//! 管理面 WebSocket：Origin 校验、`hello` 首帧、7 个 action 分发与事件广播。
//!
//! 路由 `GET /ws`：握手校验 Origin 白名单（失败 403），升级后立即推送 `hello`。
//! 每个连接一个写任务从 `mpsc::channel(256)` 取文本帧写 sink；主循环把应答与
//! 广播事件统一排队：响应用 `try_send`，队列满则发 1013 关闭。`session.state`/
//! `ports.changed`/`admin.state` 始终转发，写队列满则 1013 关闭；`debug.frame` 只
//! 转发给订阅集且 seq 去重，写队列满则静默丢弃（靠 seq 缺口提示）。广播落后
//! （`RecvError::Lagged`）按 1013 关闭。心跳每 5s 发 Ping，10s 无 Pong 判死。

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    ActionName, AdminSetParams, CloseParams, CloseResult, DebugFrameData, DebugSubscribeParams,
    DebugSubscribeResult, ErrorBody, ErrorCode, Event, HelloData, OpenParams, PortsListResult,
    Request, Response, SessionListResult, SessionSelector, WriteParams, WriteResult,
};
use crate::session::{CloseReason, ManagerError};
use crate::ws::heartbeat::Heartbeat;

use super::serve_with_nodelay;

/// 管理连接写队列容量。
const WRITE_QUEUE: usize = 256;
/// `session.write` 单次数据上限（64 KiB）。
const WRITE_MAX_BYTES: usize = 64 * 1024;
/// 管理 WS 单条消息上限（1 MiB）：在升级阶段即拒绝超大帧，避免先分配再丢弃。
const MANAGEMENT_MAX_MESSAGE: usize = 1024 * 1024;
/// 心跳 Ping 间隔。
const HEARTBEAT_PING: Duration = Duration::from_secs(5);
/// 无 Pong 判死阈值。
const HEARTBEAT_DEAD: Duration = Duration::from_secs(10);

/// 单个管理连接的调试订阅状态（主循环的读/事件转发共享）。
struct Subscription {
    selector: SessionSelector,
    /// 各会话已转发帧的最大 seq，用于回放与实时帧去重。
    seen: HashMap<String, u64>,
}

impl Default for Subscription {
    fn default() -> Self {
        Self {
            selector: SessionSelector::Ids(Vec::new()),
            seen: HashMap::new(),
        }
    }
}

impl Subscription {
    /// 会话是否在当前订阅集内。
    fn contains(&self, session_id: &str) -> bool {
        match &self.selector {
            SessionSelector::All => true,
            SessionSelector::Ids(ids) => ids.iter().any(|id| id == session_id),
        }
    }

    /// seq 去重：首次或更大 seq 返回 `true` 并记录；重复/回退返回 `false`。
    fn track(&mut self, session_id: &str, seq: u64) -> bool {
        match self.seen.get(session_id) {
            Some(&last) if seq <= last => false,
            _ => {
                self.seen.insert(session_id.to_string(), seq);
                true
            }
        }
    }
}

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
        .max_message_size(MANAGEMENT_MAX_MESSAGE)
        .on_upgrade(move |socket| async move {
            run_management_loop(app, socket).await;
        })
        .into_response()
}

/// 管理连接主循环：`hello` → 写任务 + 请求分发 + 事件转发 + 心跳。
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

    let subscription = Arc::new(Mutex::new(Subscription::default()));
    let mut event_rx = app.events.subscribe();

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

    // 心跳：连接建立即视为一次 Pong，此后每 5s 发 Ping，10s 无 Pong 判死。
    let mut heartbeat = Heartbeat::new(HEARTBEAT_PING, HEARTBEAT_DEAD);
    heartbeat.on_pong(Instant::now());
    let mut ping_tick =
        tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT_PING, HEARTBEAT_PING);

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Ok(event) => {
                        if forward_event(&subscription, &tx, event) {
                            let _ = close_tx.send(Some(1013));
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // 广播落后意味着已有事件被整批丢弃（可能含状态事件），
                        // 无法恢复顺序：按 1013 关闭，让客户端重连 + session.list 对齐。
                        let _ = close_tx.send(Some(1013));
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<Request>(&text) {
                        Ok(request) => {
                            let (response, replay) = dispatch(&app, &subscription, request).await;
                            if enqueue(&tx, response).is_err() {
                                let _ = close_tx.send(Some(1013));
                                break;
                            }
                            // 回放帧在 response 之后排队；满则丢（不挤掉状态事件）。
                            for message in replay {
                                let _ = tx.try_send(message);
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
                    Some(Ok(Message::Pong(_))) => {
                        heartbeat.on_pong(Instant::now());
                    }
                    Some(Ok(Message::Close(_))) => {
                        let _ = close_tx.send(Some(1000));
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
            _ = ping_tick.tick() => {
                let now = Instant::now();
                if heartbeat.is_dead(now) {
                    // 心跳超时：强制关闭该管理连接。
                    let _ = close_tx.send(Some(1001));
                    break;
                }
                if tx.try_send(Message::Ping(Vec::new())).is_err() {
                    // Ping 也无法入队：写队列已满，连接无法维持 liveness，关闭。
                    let _ = close_tx.send(Some(1013));
                    break;
                }
            }
        }
    }

    // 关闭发送端，让写任务排空后退出（close code 由 close 信号负责送达）。
    drop(tx);
    drop(writer);
}

/// 把一条广播事件转发到管理连接写队列；返回 `true` 表示写队列满、连接应关闭。
///
/// - `debug.frame`：仅转发给订阅集且 seq 去重（不匹配/重复不算错误）；写队列满则
///   静默丢弃（返回 `false`，靠 seq 缺口提示），不关闭连接。
/// - `session.state`/`ports.changed`/`admin.state`：始终转发；写队列满返回 `true`
///   （调用方以 1013 关闭），保证状态事件不被调试流量静默挤掉。
fn forward_event(
    subscription: &Mutex<Subscription>,
    tx: &mpsc::Sender<Message>,
    event: Event,
) -> bool {
    if let Event::DebugFrame(data) = &event {
        let mut sub = subscription.lock().unwrap();
        if !sub.contains(&data.session_id) || !sub.track(&data.session_id, data.frame.seq) {
            return false;
        }
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = tx.try_send(Message::Text(json));
        }
        return false;
    }
    let Ok(json) = serde_json::to_string(&event) else {
        return false;
    };
    tx.try_send(Message::Text(json)).is_err()
}

/// 把应答序列化后 `try_send` 进写队列；队列满返回 Err。
fn enqueue(tx: &mpsc::Sender<Message>, response: Response) -> Result<(), ()> {
    let json = serde_json::to_string(&response).map_err(|_| ())?;
    tx.try_send(Message::Text(json)).map_err(|_| ())
}

/// 按 action 分发到各处理器；返回应答与需在应答之后排队的回放帧。
async fn dispatch(
    app: &AppState,
    subscription: &Arc<Mutex<Subscription>>,
    request: Request,
) -> (Response, Vec<Message>) {
    let (result, replay): (Result<Value, ErrorBody>, Vec<Message>) = match request.action {
        ActionName::PortsList => (ports_list(app).await, Vec::new()),
        ActionName::SessionOpen => (session_open(app, &request.params).await, Vec::new()),
        ActionName::SessionClose => (session_close(app, &request.params).await, Vec::new()),
        ActionName::SessionList => (session_list(app).await, Vec::new()),
        ActionName::SessionWrite => (session_write(app, &request.params).await, Vec::new()),
        ActionName::DebugSubscribe => {
            match debug_subscribe(app, subscription, &request.params).await {
                Ok((value, frames)) => (Ok(value), frames),
                Err(error) => (Err(error), Vec::new()),
            }
        }
        ActionName::AdminSet => (admin_set(app, &request.params).await, Vec::new()),
        ActionName::Unknown => (
            Err(ErrorBody::new(ErrorCode::UnknownAction, "未知 action")),
            Vec::new(),
        ),
    };
    let response = match result {
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
    };
    (response, replay)
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
        autostart: app.admin.autostart(),
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

/// `debug.subscribe`：更新订阅集，`replay:true` 时回放环形缓冲（seq 去重）。
///
/// 回放帧不在此处入队，而是随返回一并交给调用方，在应答之后排队，避免回放
/// 帧抢先于应答、也避免与实时帧乱序。
async fn debug_subscribe(
    app: &AppState,
    subscription: &Arc<Mutex<Subscription>>,
    params: &Value,
) -> Result<(Value, Vec<Message>), ErrorBody> {
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

    let mut replay = Vec::new();
    {
        let mut sub = subscription.lock().unwrap();
        sub.selector = params.session_ids.clone();
        if params.replay {
            for id in &subscribed {
                if let Some(frames) = app.manager.debug_snapshot(id) {
                    for frame in frames {
                        if !sub.track(id, frame.seq) {
                            continue;
                        }
                        let data = DebugFrameData {
                            session_id: id.clone(),
                            frame: frame.with_replay(true),
                        };
                        if let Ok(json) = serde_json::to_string(&Event::DebugFrame(data)) {
                            replay.push(Message::Text(json));
                        }
                    }
                }
            }
        }
    }

    let value = to_value(DebugSubscribeResult { subscribed })?;
    Ok((value, replay))
}

/// `admin.set`：运行时开关管理页服务（不回写配置），并广播 `admin.state`。
async fn admin_set(app: &AppState, params: &Value) -> Result<Value, ErrorBody> {
    let params: AdminSetParams = parse_params(params)?;
    let state = app.admin.set_enabled(params.enabled).await;
    let _ = app.events.send(Event::AdminState(state.clone()));
    to_value(state)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{DebugFrame, PortsChangedData};

    /// 写队列满时：`debug.frame` 丢弃（返回 false），状态事件关闭（返回 true）。
    #[test]
    fn forward_event_drops_debug_on_full_but_closes_on_state_full() {
        let (tx, _rx) = mpsc::channel::<Message>(1);
        let subscription = Mutex::new(Subscription {
            selector: SessionSelector::All,
            seen: HashMap::new(),
        });
        // 塞满容量为 1 的写队列。
        assert!(tx.try_send(Message::Text("full".into())).is_ok());

        // debug.frame 满 → 丢弃，返回 false（不关闭连接）。
        let debug = Event::DebugFrame(DebugFrameData {
            session_id: "s1".into(),
            frame: DebugFrame::new("rx", 1, 0, vec![0u8; 4]),
        });
        assert!(!forward_event(&subscription, &tx, debug));

        // 状态事件满 → 返回 true（调用方以 1013 关闭）。
        let state = Event::PortsChanged(PortsChangedData {
            added: vec![],
            removed: vec![],
        });
        assert!(forward_event(&subscription, &tx, state));
    }
}
