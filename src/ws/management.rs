//! 管理面 WebSocket：Origin 校验、`hello` 首帧、7 个 action 分发与响应队列。
//!
//! 路由 `GET /ws`：握手校验 Origin 白名单（失败 403），升级后立即推送 `hello`。
//! 每个连接一个写任务从 `mpsc::channel(256)` 取文本帧写 sink；响应用 `try_send`，
//! 队列满则发 1013 关闭；一个读循环解析 [`Request`] 并按 action 分发，应答带相同 `id`。
//! 状态/端口/管理/调试事件（任务 11）与心跳（任务 11）后续接入。

use std::borrow::Cow;
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response as AxumResponse};
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};

use crate::app::AppState;
use crate::config::OriginAllowlist;
use crate::protocol::{
    ActionName, AdminSetParams, CloseParams, CloseResult, DebugSubscribeParams,
    DebugSubscribeResult, ErrorBody, ErrorCode, Event, HelloData, OpenParams, PortsListResult,
    Request, Response, SessionListResult, SessionSelector, WriteParams, WriteResult,
};
use crate::session::{CloseReason, ManagerError};

use super::serve_with_nodelay;

/// 管理连接写队列容量。
const WRITE_QUEUE: usize = 256;
/// `session.write` 单次数据上限（64 KiB）。
const WRITE_MAX_BYTES: usize = 64 * 1024;

/// 管理 WS 路由状态。
struct WsState {
    app: Arc<AppState>,
    origins: OriginAllowlist,
}

/// 构造管理 WS 路由（`GET /ws`）。
fn router(state: Arc<AppState>) -> Router {
    let origins = OriginAllowlist::new(&state.config.security.allowed_origins);
    let state = Arc::new(WsState {
        app: state,
        origins,
    });
    Router::new().route(
        "/ws",
        get(move |headers: HeaderMap, upgrade: WebSocketUpgrade| {
            let state = Arc::clone(&state);
            async move { upgrade_management(state, headers, upgrade).await }
        }),
    )
}

/// 启动管理 WS accept 循环。
pub(crate) async fn serve(
    state: Arc<AppState>,
    listener: TcpListener,
    shutdown: oneshot::Receiver<()>,
) {
    serve_with_nodelay(listener, router(state), shutdown).await;
}

/// 握手阶段：校验 Origin 后升级或返回 403。
async fn upgrade_management(
    state: Arc<WsState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> AxumResponse {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if !state.origins.allows(origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let app = Arc::clone(&state.app);
    upgrade
        .on_upgrade(move |socket| async move {
            run_management_loop(app, socket).await;
        })
        .into_response()
}

/// 管理连接主循环：`hello` → 写任务 + 读循环。
async fn run_management_loop(app: Arc<AppState>, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();

    // 升级后立即推送 hello。
    let hello =
        serde_json::to_string(&Event::Hello(hello_data(&app))).expect("hello 序列化不应失败");
    if sender.send(Message::Text(hello)).await.is_err() {
        return;
    }

    let (tx, mut rx) = mpsc::channel::<Message>(WRITE_QUEUE);
    let (close_tx, mut close_rx) = watch::channel::<Option<u16>>(None);

    // 写任务：从队列取帧写 sink；收到关闭信号时发对应 close code。
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = close_rx.changed() => {
                    let code = *close_rx.borrow();
                    if let Some(code) = code {
                        let _ = sender.send(close_message(code)).await;
                    }
                    return;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(message) => {
                            if sender.send(message).await.is_err() {
                                return;
                            }
                        }
                        None => return,
                    }
                }
            }
        }
    });

    // debug.subscribe 的本连接订阅集（任务 11 据此推调试帧）。
    let mut subscription = SessionSelector::Ids(Vec::new());

    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<Request>(&text) {
                Ok(request) => {
                    let response = dispatch(&app, &mut subscription, request).await;
                    if enqueue(&tx, response).is_err() {
                        let _ = close_tx.send(Some(1013));
                        break;
                    }
                }
                Err(_) => {
                    let response = Response {
                        id: String::new(),
                        ok: false,
                        data: None,
                        error: Some(ErrorBody::new(ErrorCode::InvalidRequest, "请求格式错误")),
                    };
                    if enqueue(&tx, response).is_err() {
                        let _ = close_tx.send(Some(1013));
                        break;
                    }
                }
            },
            Some(Ok(Message::Ping(payload))) => {
                let _ = tx.try_send(Message::Pong(payload));
            }
            Some(Ok(Message::Close(_))) => {
                let _ = close_tx.send(Some(1000));
                break;
            }
            Some(Ok(_)) => {}
            Some(Err(_)) | None => break,
        }
    }

    // 关闭发送端，让写任务排空后退出（close code 由 close 信号负责送达）。
    drop(tx);
    drop(writer);
}

/// 把应答序列化后 `try_send` 进写队列；队列满返回 Err。
fn enqueue(tx: &mpsc::Sender<Message>, response: Response) -> Result<(), ()> {
    let json = serde_json::to_string(&response).map_err(|_| ())?;
    tx.try_send(Message::Text(json)).map_err(|_| ())
}

/// 按 action 分发到各处理器；错误统一为 [`ErrorBody`]。
async fn dispatch(
    app: &AppState,
    subscription: &mut SessionSelector,
    request: Request,
) -> Response {
    let result: Result<Value, ErrorBody> = match request.action {
        ActionName::PortsList => ports_list(app).await,
        ActionName::SessionOpen => session_open(app, &request.params).await,
        ActionName::SessionClose => session_close(app, &request.params).await,
        ActionName::SessionList => session_list(app).await,
        ActionName::SessionWrite => session_write(app, &request.params).await,
        ActionName::DebugSubscribe => debug_subscribe(app, subscription, &request.params).await,
        ActionName::AdminSet => admin_set(app, &request.params).await,
        ActionName::Unknown => Err(ErrorBody::new(ErrorCode::UnknownAction, "未知 action")),
    };
    match result {
        Ok(data) => Response {
            id: request.id,
            ok: true,
            data: Some(data),
            error: None,
        },
        Err(error) => Response {
            id: request.id,
            ok: false,
            data: None,
            error: Some(error),
        },
    }
}

/// `hello` 事件数据。
fn hello_data(app: &AppState) -> HelloData {
    HelloData {
        protocol: "ws2s/v1".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        management_port: app.config.server.management_port,
        admin: app.admin.status(),
        // 任务 14 接入真实自启状态。
        autostart: false,
    }
}

/// `ports.list`：立即重扫并返回当前串口快照。
async fn ports_list(app: &AppState) -> Result<Value, ErrorBody> {
    let ports = app
        .scanner
        .scan()
        .map_err(|e| ErrorBody::new(ErrorCode::Internal, e.to_string()))?;
    to_value(PortsListResult { ports })
}

/// `session.open`：打开串口或复用同端口非 active 会话。
async fn session_open(app: &AppState, params: &Value) -> Result<Value, ErrorBody> {
    let params: OpenParams = parse_params(params)?;
    let result = app.manager.open(params).await.map_err(manager_error)?;
    to_value(result)
}

/// `session.close`：幂等关闭会话并回收端口。
async fn session_close(app: &AppState, params: &Value) -> Result<Value, ErrorBody> {
    let params: CloseParams = parse_params(params)?;
    app.manager
        .close(&params.session_id, CloseReason::AdminClosed)
        .await
        .map_err(manager_error)?;
    to_value(CloseResult { closed: true })
}

/// `session.list`：会话列表快照（不含 dataUrl/token）。
async fn session_list(app: &AppState) -> Result<Value, ErrorBody> {
    let sessions = app.manager.list().await;
    to_value(SessionListResult { sessions })
}

/// `session.write`：仅 active；base64 解码、≤64 KiB、50ms 内送入 tx 队列。
async fn session_write(app: &AppState, params: &Value) -> Result<Value, ErrorBody> {
    let params: WriteParams = parse_params(params)?;
    let data = STANDARD
        .decode(&params.data_b64)
        .map_err(|_| ErrorBody::new(ErrorCode::InvalidParams, "dataB64 不是合法 base64"))?;
    if data.len() > WRITE_MAX_BYTES {
        return Err(ErrorBody::new(
            ErrorCode::InvalidParams,
            "调试写单次上限 64 KiB",
        ));
    }
    let written = app
        .manager
        .write(&params.session_id, data)
        .await
        .map_err(manager_error)?;
    to_value(WriteResult { written })
}

/// `debug.subscribe`：记录本连接订阅集并返回当前已订阅会话（任务 11 推帧）。
async fn debug_subscribe(
    app: &AppState,
    subscription: &mut SessionSelector,
    params: &Value,
) -> Result<Value, ErrorBody> {
    let params: DebugSubscribeParams = parse_params(params)?;
    let existing = app.manager.list().await;
    let subscribed: Vec<String> = match &params.session_ids {
        SessionSelector::All => existing.iter().map(|s| s.session_id.clone()).collect(),
        SessionSelector::Ids(ids) => ids
            .iter()
            .filter(|id| existing.iter().any(|s| &s.session_id == *id))
            .cloned()
            .collect(),
    };
    *subscription = params.session_ids;
    to_value(DebugSubscribeResult { subscribed })
}

/// `admin.set`：运行时开关管理页服务（不回写配置）。
async fn admin_set(app: &AppState, params: &Value) -> Result<Value, ErrorBody> {
    let params: AdminSetParams = parse_params(params)?;
    to_value(app.admin.set_enabled(params.enabled))
}

/// 把 params 反序列化为目标类型；失败返回 [`ErrorCode::InvalidParams`]。
fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, ErrorBody> {
    serde_json::from_value(params.clone())
        .map_err(|_| ErrorBody::new(ErrorCode::InvalidParams, "参数缺失或取值非法"))
}

/// [`ManagerError`] → [`ErrorBody`]，用 `to_code()` 映射错误码。
fn manager_error(err: ManagerError) -> ErrorBody {
    ErrorBody::new(err.to_code(), err.to_string())
}

/// 序列化成功结果；失败（不应发生）落 [`ErrorCode::Internal`]。
fn to_value<T: serde::Serialize>(value: T) -> Result<Value, ErrorBody> {
    serde_json::to_value(value).map_err(|_| ErrorBody::new(ErrorCode::Internal, "应答序列化失败"))
}

/// 构造一个无 reason 的 Close 消息。
fn close_message(code: u16) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: Cow::Borrowed(""),
    }))
}
