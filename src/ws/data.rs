//! 数据面 WebSocket：握手校验（Origin/token）、attach、二进制透传与 close code。
//!
//! 每个会话在 `session.open` 时通过 [`AxumDataListenerFactory`] 绑定一个监听
//! `127.0.0.1:0` 的数据 WS 服务，路由 `GET /ws/:token`。握手时校验 Origin
//! 白名单与 token；`awaiting`/`grace` 时 attach 进入 `active`，已 `active` 时以
//! 4002 关闭。数据循环把客户端二进制帧送入串口写队列、把串口读到的字节块作为
//! 二进制帧回发；文本 1003、超 1 MiB 1009；会话关闭 watch 触发时发对应 close code。
//!
//! 为避免 manager 与数据监听器工厂的强循环，工厂仅持有 `Weak<SessionManager>`，
//! 每个连接处理时临时 `upgrade` 取得 `Arc`，不长期持有 manager。

use std::borrow::Cow;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};

use crate::config::OriginAllowlist;
use crate::protocol::{CloseCode, SessionState};
use crate::session::{
    AttachError, CloseReason, DataError, DataListenerFactory, DataListenerHandle, Session,
    SessionManager,
};
use crate::ws::heartbeat::Heartbeat;

use super::serve_with_nodelay;

/// 心跳 Ping 间隔。
const HEARTBEAT_PING: Duration = Duration::from_secs(5);
/// 无 Pong 判死阈值。
const HEARTBEAT_DEAD: Duration = Duration::from_secs(10);

/// 数据 WS 单监听器的共享状态（随服务任务存活）。
struct WsState {
    session: Arc<Session>,
    manager: Weak<SessionManager>,
    origins: OriginAllowlist,
    /// 单帧字节上限（= `tx_queue_bytes`，默认 1 MiB）。
    max_frame_bytes: usize,
}

/// 真实数据监听器工厂：绑定 `127.0.0.1:0`，路由 `GET /ws/:token`。
///
/// 持有 [`Weak<SessionManager>`] 以在连接处理时调用 `attach`/`enter_grace`/`close`，
/// 保证状态事件与端口清理走 manager 单一路径，同时不构成 manager↔factory 强循环。
pub struct AxumDataListenerFactory {
    manager: Weak<SessionManager>,
    origins: OriginAllowlist,
    max_frame_bytes: usize,
}

impl AxumDataListenerFactory {
    /// 构造工厂。`manager` 为尚未完成的 `SessionManager` 的弱引用（配合
    /// `Arc::new_cyclic` 传入），`origins` 为 Origin 白名单，`max_frame_bytes`
    /// 为单帧字节上限（取 `SessionConfig::tx_queue_bytes`）。
    pub fn new(
        manager: Weak<SessionManager>,
        origins: OriginAllowlist,
        max_frame_bytes: usize,
    ) -> Self {
        Self {
            manager,
            origins,
            max_frame_bytes,
        }
    }
}

impl DataListenerFactory for AxumDataListenerFactory {
    fn bind(&self, session: Arc<Session>) -> Result<DataListenerHandle, DataError> {
        // 同步绑定（bind 系统调用开销小，且 `bind` 由 manager 在运行时上下文中调用）。
        let std_listener =
            std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(DataError::Bind)?;
        std_listener
            .set_nonblocking(true)
            .map_err(DataError::Bind)?;
        let listener = tokio::net::TcpListener::from_std(std_listener).map_err(DataError::Bind)?;
        let port = listener.local_addr().map_err(DataError::Bind)?.port();

        let state = Arc::new(WsState {
            session,
            manager: self.manager.clone(),
            origins: self.origins.clone(),
            max_frame_bytes: self.max_frame_bytes,
        });

        let router = Router::new().route(
            "/ws/:token",
            get({
                let state = Arc::clone(&state);
                move |Path(token): Path<String>, headers: HeaderMap, upgrade: WebSocketUpgrade| {
                    let state = Arc::clone(&state);
                    async move { data_ws_upgrade(state, token, headers, upgrade).await }
                }
            }),
        );

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            serve_with_nodelay(listener, router, shutdown_rx).await;
        });

        Ok(DataListenerHandle {
            port,
            shutdown: Box::new(move || {
                let _ = shutdown_tx.send(());
            }),
        })
    }
}

/// 握手阶段：校验 Origin/token 后决定升级或返回错误响应。
async fn data_ws_upgrade(
    state: Arc<WsState>,
    token: String,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if !state.origins.allows(origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !state.session.check_token(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if state.session.state() == SessionState::Closed {
        return StatusCode::NOT_FOUND.into_response();
    }

    let session_id = state.session.id().to_string();
    upgrade
        .on_upgrade(move |socket| async move {
            run_data_loop(state, session_id, token, socket).await;
        })
        .into_response()
}

/// 数据循环：attach 后双向透传，处理文本/超大帧/正常关闭/异常断开与会话关闭。
async fn run_data_loop(
    state: Arc<WsState>,
    session_id: String,
    token: String,
    mut socket: WebSocket,
) {
    let manager = match state.manager.upgrade() {
        Some(manager) => manager,
        None => return,
    };

    let attached = match manager.attach(&session_id, &token).await {
        Ok(attached) => attached,
        Err(AttachError::AlreadyActive) => {
            // 会话已 active（第二客户端）：升级后以 4002 关闭。
            let _ = socket
                .send(close_message(CloseCode::PortBusy.as_u16()))
                .await;
            return;
        }
        Err(_) => return,
    };

    let mut rx = attached.rx;
    let tx = attached.tx;
    let mut close_watch = state.session.close_subscribe();

    // attach 与建 watch 之间会话可能已关闭：先读当前值补发 close code。
    if let Some(code) = state
        .session
        .close_reason()
        .and_then(CloseReason::close_code)
    {
        let _ = socket.send(close_message(code)).await;
        return;
    }

    let (mut sender, mut receiver) = socket.split();

    // 心跳：连接建立即视为一次 Pong，此后每 5s 发 Ping，10s 无 Pong 判死。
    let mut heartbeat = Heartbeat::new(HEARTBEAT_PING, HEARTBEAT_DEAD);
    heartbeat.on_pong(Instant::now());
    let mut ping_tick =
        tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT_PING, HEARTBEAT_PING);

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Binary(data))) => {
                        if data.len() > state.max_frame_bytes {
                            let _ = sender.send(close_message(CloseCode::MessageTooBig.as_u16())).await;
                            state.session.return_rx(rx);
                            let _ = manager.enter_grace(&session_id).await;
                            return;
                        }
                        // 容量 1 的 tx 通道：send 阻塞即自然背压；期间仍轮询 close watch，
                        // 避免写停滞/掉线/背压触发会话关闭时 close code 不送达。
                        tokio::select! {
                            res = tx.send(data) => {
                                if res.is_err() {
                                    send_session_close(&mut sender, &state.session).await;
                                    return;
                                }
                            }
                            _ = close_watch.changed() => {
                                send_session_close(&mut sender, &state.session).await;
                                return;
                            }
                        }
                    }
                    Some(Ok(Message::Text(_))) => {
                        let _ = sender.send(close_message(CloseCode::UnsupportedData.as_u16())).await;
                        state.session.return_rx(rx);
                        let _ = manager.enter_grace(&session_id).await;
                        return;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sender.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        heartbeat.on_pong(Instant::now());
                    }
                    Some(Ok(Message::Close(frame))) => {
                        // 无 code 或 1000 视为正常关闭；其余视为异常断开。
                        let normal = frame.as_ref().map(|f| f.code == 1000).unwrap_or(true);
                        if normal {
                            let _ = sender.send(close_message(1000)).await;
                            let _ = manager.close(&session_id, CloseReason::ClientClosed).await;
                        } else {
                            state.session.return_rx(rx);
                            let _ = manager.enter_grace(&session_id).await;
                        }
                        return;
                    }
                    Some(Err(_)) | None => {
                        // 客户端异常断开：归还接收器并进入宽限（细节见任务 9）。
                        state.session.return_rx(rx);
                        let _ = manager.enter_grace(&session_id).await;
                        return;
                    }
                }
            }
            outgoing = rx.recv() => {
                match outgoing {
                    Some(bytes) => {
                        // 客户端慢收时仍轮询 close watch，避免 close code 延迟。
                        tokio::select! {
                            res = sender.send(Message::Binary(bytes)) => {
                                if res.is_err() {
                                    state.session.return_rx(rx);
                                    let _ = manager.enter_grace(&session_id).await;
                                    return;
                                }
                            }
                            _ = close_watch.changed() => {
                                send_session_close(&mut sender, &state.session).await;
                                return;
                            }
                        }
                    }
                    None => {
                        // 串口→客户端通道关闭（会话已关、IO 拆除）：补发 close code，不静默 EOF。
                        send_session_close(&mut sender, &state.session).await;
                        return;
                    }
                }
            }
            _ = close_watch.changed() => {
                // 会话因设备掉线/写停滞/背压/管理关闭而关闭：发对应 close code 后退出。
                send_session_close(&mut sender, &state.session).await;
                return;
            }
            _ = ping_tick.tick() => {
                let now = Instant::now();
                if heartbeat.is_dead(now) {
                    // 心跳超时：按异常断开进入宽限（数据连接走 4001/grace）。
                    state.session.return_rx(rx);
                    let _ = manager.enter_grace(&session_id).await;
                    return;
                }
                let _ = sender.send(Message::Ping(Vec::new())).await;
            }
        }
    }
}

/// 从会话关闭原因取 close code 补发 close frame；无 code 时用 1001（GoingAway）。
async fn send_session_close(sender: &mut SplitSink<WebSocket, Message>, session: &Session) {
    let code = session
        .close_reason()
        .and_then(CloseReason::close_code)
        .unwrap_or(CloseCode::GoingAway.as_u16());
    let _ = sender.send(close_message(code)).await;
}

/// 构造一个无 reason 的 Close 消息。
fn close_message(code: u16) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: Cow::Borrowed(""),
    }))
}
