//! 会话管理器：端口独占、open/close/list、复用、attach 与超时定时器。
//!
//! 持有串口工厂、数据监听器工厂、事件广播、端口锁与会话表。会话自身因 I/O
//! 关闭时通过 `closed_tx` 通知，manager 的 `closed_rx` 任务统一走 [`close`]
//! 清理，避免重复事件。
//!
//! [`close`]: SessionManager::close

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};

use crate::config::SessionConfig;
use crate::protocol::{
    ErrorCode, Event, OpenParams, OpenResult, SessionInfo, SessionState, SessionStateData,
};
use crate::serial::{ParamError, SerialParams, TransportError, TransportFactory};

use super::data_listener::{DataListenerFactory, DataListenerHandle};
use super::session::{AttachedData, CloseReason, Session, SessionError};

/// 数据监听绑定失败、串口打开失败、端口/会话状态等管理器级错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManagerError {
    #[error("串口参数非法：{0}")]
    InvalidParams(String),
    #[error("串口不存在")]
    PortNotFound,
    #[error("串口被占用，或同端口已有 active 会话")]
    PortBusy,
    #[error("无权限打开串口")]
    PermissionDenied,
    #[error("打开串口失败：{0}")]
    SerialOpenFailed(String),
    #[error("会话不存在或已关闭")]
    SessionNotFound,
    #[error("当前状态不允许该操作")]
    SessionState,
    #[error("超过最大会话数")]
    LimitExceeded,
}

impl ManagerError {
    /// 映射到管理协议错误码（任务 10 使用）。
    pub fn to_code(&self) -> ErrorCode {
        match self {
            ManagerError::InvalidParams(_) => ErrorCode::InvalidParams,
            ManagerError::PortNotFound => ErrorCode::PortNotFound,
            ManagerError::PortBusy => ErrorCode::PortBusy,
            ManagerError::PermissionDenied => ErrorCode::PermissionDenied,
            ManagerError::SerialOpenFailed(_) => ErrorCode::SerialOpenFailed,
            ManagerError::SessionNotFound => ErrorCode::SessionNotFound,
            ManagerError::SessionState => ErrorCode::SessionState,
            ManagerError::LimitExceeded => ErrorCode::LimitExceeded,
        }
    }
}

/// `attach` 失败：token 不匹配、会话不存在或状态不允许。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachError {
    #[error("token 无效或已轮换")]
    InvalidToken,
    #[error("会话不存在或已关闭")]
    SessionNotFound,
    #[error("会话已 active，无法重复附加")]
    AlreadyActive,
    #[error("当前状态不允许附加")]
    SessionState,
}

impl From<SessionError> for AttachError {
    fn from(err: SessionError) -> Self {
        match err {
            SessionError::AlreadyActive => AttachError::AlreadyActive,
            SessionError::Closed | SessionError::NotActive | SessionError::NotGrace => {
                AttachError::SessionState
            }
        }
    }
}

/// 单个会话的管理句柄：数据监听器 shutdown 与超时定时器。
struct SessionHandle {
    shutdown: Option<Box<dyn FnOnce() + Send>>,
    deadline: Arc<Deadline>,
}

/// 可重入的超时截止：定时任务到点前会重新读取截止，若被刷新则继续等待，
/// 避免晚到的刷新被旧 timer 杀掉。
struct Deadline {
    target: Mutex<Option<(tokio::time::Instant, CloseReason)>>,
}

impl Deadline {
    fn new() -> Self {
        Self {
            target: Mutex::new(None),
        }
    }

    /// 设置/刷新截止时间与到点后的关闭原因。
    fn set(&self, at: tokio::time::Instant, reason: CloseReason) {
        *self.target.lock().unwrap() = Some((at, reason));
    }

    /// 取消截止（attach/close 时调用）。
    fn cancel(&self) {
        *self.target.lock().unwrap() = None;
    }
}

/// manager 内部共享状态（不含 `closed_tx`，避免清理任务持有发送端造成泄漏）。
struct ManagerShared {
    transport_factory: Arc<dyn TransportFactory>,
    listener_factory: Arc<dyn DataListenerFactory>,
    cfg: SessionConfig,
    events: broadcast::Sender<Event>,
    ports: Mutex<HashMap<String, String>>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    handles: Mutex<HashMap<String, SessionHandle>>,
}

struct ManagerInner {
    shared: Arc<ManagerShared>,
    closed_tx: mpsc::UnboundedSender<String>,
}

/// 会话管理器：独占串口生命周期。
pub struct SessionManager {
    inner: Arc<ManagerInner>,
}

impl SessionManager {
    /// 构造管理器并启动 `closed_rx` 清理任务。
    pub fn new(
        transport_factory: Arc<dyn TransportFactory>,
        listener_factory: Arc<dyn DataListenerFactory>,
        cfg: SessionConfig,
        events: broadcast::Sender<Event>,
    ) -> Self {
        let shared = Arc::new(ManagerShared {
            transport_factory,
            listener_factory,
            cfg,
            events,
            ports: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            handles: Mutex::new(HashMap::new()),
        });
        let (closed_tx, mut closed_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(ManagerInner { shared, closed_tx });

        // 消费所有会话的 `closed_rx`：设备掉线/写停滞/背压等由会话自身触发，
        // 这里统一走 `close_inner` 清理（从 close watch 取真实原因），保证端口被释放。
        {
            let shared = Arc::clone(&inner.shared);
            tokio::spawn(async move {
                while let Some(session_id) = closed_rx.recv().await {
                    let reason = shared
                        .sessions
                        .lock()
                        .unwrap()
                        .get(&session_id)
                        .and_then(|session| session.close_reason())
                        .unwrap_or(CloseReason::DeviceLost);
                    shared.close_inner(&session_id, reason);
                }
            });
        }

        SessionManager { inner }
    }

    /// 打开串口或复用同端口的非 active 会话。
    pub async fn open(&self, params: OpenParams) -> Result<OpenResult, ManagerError> {
        let serial: SerialParams = params.into();
        serial.validate().map_err(map_param_error)?;

        let shared = Arc::clone(&self.inner.shared);
        let path = serial.path.clone();

        // 先查端口锁：不存在才需要真正打开串口。
        let existing_id = {
            let ports = shared.ports.lock().unwrap();
            ports.get(&path).cloned()
        };

        if let Some(session_id) = existing_id {
            let session = shared
                .sessions
                .lock()
                .unwrap()
                .get(&session_id)
                .cloned()
                .ok_or(ManagerError::SessionNotFound)?;
            match session.state() {
                SessionState::Awaiting | SessionState::Grace => {
                    return ManagerShared::reuse(&shared, &session).await;
                }
                SessionState::Active => return Err(ManagerError::PortBusy),
                SessionState::Closed => return Err(ManagerError::SessionNotFound),
            }
        }

        // 新会话：先检查最大会话数。
        {
            let sessions = shared.sessions.lock().unwrap();
            if sessions.len() >= shared.cfg.max_sessions {
                return Err(ManagerError::LimitExceeded);
            }
        }

        // 打开串口（阻塞调用放到 spawn_blocking）。
        let transport = {
            let factory = Arc::clone(&shared.transport_factory);
            let serial = serial.clone();
            tokio::task::spawn_blocking(move || factory.open(&serial))
                .await
                .map_err(|e| ManagerError::SerialOpenFailed(e.to_string()))?
                .map_err(map_transport_error)?
        };

        // 生成会话并绑定数据监听器。
        let id = uuid::Uuid::new_v4().simple().to_string();
        let session = Arc::new(Session::with_transport(
            id.clone(),
            serial,
            shared.cfg.clone(),
            shared.events.clone(),
            self.inner.closed_tx.clone(),
            Some(transport),
        ));
        let handle = match shared.listener_factory.bind(Arc::clone(&session)) {
            Ok(handle) => handle,
            Err(e) => {
                // 监听绑定失败：丢弃串口传输释放端口。
                drop(session.take_transport());
                return Err(ManagerError::SerialOpenFailed(e.to_string()));
            }
        };
        let port = handle.port;
        session.set_data_port(port);

        // 插入会话表 + 端口锁 + 句柄。
        {
            let mut sessions = shared.sessions.lock().unwrap();
            sessions.insert(id.clone(), Arc::clone(&session));
        }
        {
            let mut ports = shared.ports.lock().unwrap();
            ports.insert(path, id.clone());
        }
        let deadline = Arc::new(Deadline::new());
        {
            let mut handles = shared.handles.lock().unwrap();
            handles.insert(
                id.clone(),
                SessionHandle {
                    shutdown: Some(handle.shutdown),
                    deadline: Arc::clone(&deadline),
                },
            );
        }

        // 发送 awaiting 事件并启动 5s 定时器。
        let connect_timeout_ms = shared.cfg.connect_timeout_ms;
        shared.emit(&session, SessionState::Awaiting, None, None);
        deadline.set(
            tokio::time::Instant::now() + Duration::from_millis(connect_timeout_ms),
            CloseReason::AwaitingTimeout,
        );
        ManagerShared::spawn_timer(Arc::clone(&shared), id.clone(), deadline);

        Ok(OpenResult {
            session_id: id,
            data_url: data_url(port, session.current_token()),
            connect_timeout_ms,
            reused: false,
            counters: session.counters(),
        })
    }

    /// 关闭会话（幂等）：移除会话、释放端口、关闭数据监听、发送 `session.state(closed)`。
    pub async fn close(&self, session_id: &str, reason: CloseReason) -> Result<(), ManagerError> {
        self.inner.shared.close_inner(session_id, reason);
        Ok(())
    }

    /// 会话列表快照。
    pub async fn list(&self) -> Vec<SessionInfo> {
        let sessions = self.inner.shared.sessions.lock().unwrap();
        let mut infos: Vec<SessionInfo> = sessions.values().map(|s| s.info()).collect();
        infos.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        infos
    }

    /// 校验 token 并把会话切到 active，返回数据 WS 收发通道。
    pub async fn attach(&self, session_id: &str, token: &str) -> Result<AttachedData, AttachError> {
        let shared = Arc::clone(&self.inner.shared);
        let session = shared
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .ok_or(AttachError::SessionNotFound)?;
        if !session.check_token(token) {
            return Err(AttachError::InvalidToken);
        }
        let attached = session.attach()?;
        session.clear_connect_by();
        // 进入 active：取消 awaiting/grace 超时定时器并发送状态事件。
        shared.cancel_deadline(session_id);
        shared.emit(&session, SessionState::Active, None, None);
        Ok(attached)
    }

    /// 数据 WS 异常断开：`active` → `grace`，发事件并启动 10s 宽限定时器。
    pub async fn enter_grace(&self, session_id: &str) -> Result<(), ManagerError> {
        let shared = Arc::clone(&self.inner.shared);
        let session = shared
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .ok_or(ManagerError::SessionNotFound)?;
        session
            .enter_grace()
            .map_err(|_| ManagerError::SessionState)?;
        shared.emit(&session, SessionState::Grace, None, None);
        if let Some(deadline) = shared.set_deadline(
            session_id,
            Duration::from_millis(shared.cfg.grace_ms),
            CloseReason::GraceTimeout,
        ) {
            ManagerShared::spawn_timer(Arc::clone(&shared), session_id.to_string(), deadline);
        }
        Ok(())
    }
}

impl ManagerShared {
    /// 复用 awaiting/grace 会话：关闭旧监听器、轮换 token、绑定新监听器、重置截止。
    async fn reuse(
        shared: &Arc<ManagerShared>,
        session: &Arc<Session>,
    ) -> Result<OpenResult, ManagerError> {
        let session_id = session.id().to_string();
        let new_token = session.rotate_token();

        // 绑定新监听器（`replace_listener` 会关闭旧监听器并保留原 deadline）。
        let handle = shared
            .listener_factory
            .bind(Arc::clone(session))
            .map_err(|e| ManagerError::SerialOpenFailed(e.to_string()))?;
        let port = handle.port;
        session.set_data_port(port);
        shared.replace_listener(&session_id, handle);

        // 复用 awaiting 重置 5s 连接截止；复用 grace 重置 10s 宽限截止。
        // 原 timer 到点前会重新读取截止，无需额外启动新 timer。
        let (dur, reason) = match session.state() {
            SessionState::Grace => (shared.cfg.grace_ms, CloseReason::GraceTimeout),
            _ => {
                session.refresh_connect_by();
                (shared.cfg.connect_timeout_ms, CloseReason::AwaitingTimeout)
            }
        };
        shared.set_deadline(&session_id, Duration::from_millis(dur), reason);

        Ok(OpenResult {
            session_id,
            data_url: data_url(port, new_token),
            connect_timeout_ms: shared.cfg.connect_timeout_ms,
            reused: true,
            counters: session.counters(),
        })
    }

    /// 关闭会话并清理资源；已不存在时静默返回（幂等）。
    fn close_inner(&self, session_id: &str, reason: CloseReason) {
        let (session, handle) = {
            let mut sessions = self.sessions.lock().unwrap();
            let mut handles = self.handles.lock().unwrap();
            match (sessions.remove(session_id), handles.remove(session_id)) {
                (Some(session), handle) => (session, handle),
                (None, _) => return,
            }
        };
        // 释放端口锁。
        {
            let mut ports = self.ports.lock().unwrap();
            let path = session.info().port;
            ports.remove(&path);
        }
        // 触发 close watch / closed_tx（幂等；先移除再 close 以避免重复事件）。
        session.close(reason);
        // 发送 `session.state(closed)`。
        self.emit(
            &session,
            SessionState::Closed,
            Some(reason),
            reason.close_code(),
        );
        // 关闭数据监听器、取消定时器、释放串口传输。
        if let Some(handle) = handle {
            handle.deadline.cancel();
            if let Some(shutdown) = handle.shutdown {
                shutdown();
            }
        }
        drop(session.take_transport());
    }

    /// 向事件广播发送 `session.state`；无订阅者时静默丢弃。
    fn emit(
        &self,
        session: &Arc<Session>,
        state: SessionState,
        reason: Option<CloseReason>,
        close_code: Option<u16>,
    ) {
        let info = session.info();
        let data = SessionStateData {
            session_id: info.session_id,
            state,
            port: info.port,
            params: info.params,
            reason: reason.map(|r| r.as_str().to_string()),
            close_code,
            grace_ends_at: info.grace_ends_at,
            counters: info.counters,
            last_gap: info.last_gap,
            last_error: info.last_error,
            last_rx_at: info.last_rx_at,
            last_tx_at: info.last_tx_at,
        };
        let _ = self.events.send(Event::SessionState(data));
    }

    /// 刷新某会话的超时截止并返回定时器句柄（调用方据此启动/唤醒定时任务）。
    fn set_deadline(
        &self,
        session_id: &str,
        dur: Duration,
        reason: CloseReason,
    ) -> Option<Arc<Deadline>> {
        let deadline = {
            let handles = self.handles.lock().unwrap();
            handles
                .get(session_id)
                .map(|handle| handle.deadline.clone())
        };
        if let Some(deadline) = &deadline {
            deadline.set(tokio::time::Instant::now() + dur, reason);
        }
        deadline
    }

    /// 取消某会话的超时截止。
    fn cancel_deadline(&self, session_id: &str) {
        let handles = self.handles.lock().unwrap();
        if let Some(handle) = handles.get(session_id) {
            handle.deadline.cancel();
        }
    }

    /// 启动定时任务：到点前重新读取截止，若被刷新则继续等待。
    ///
    /// 采用可重入 deadline：每次到点（`sleep_until` 返回）都重新读取截止；
    /// 若截止被复用/宽限刷新到更晚，则继续等新截止，避免旧 timer 提前杀会话。
    fn spawn_timer(shared: Arc<ManagerShared>, session_id: String, deadline: Arc<Deadline>) {
        tokio::spawn(async move {
            loop {
                let (at, reason) = match *deadline.target.lock().unwrap() {
                    Some(target) => target,
                    None => return,
                };
                let now = tokio::time::Instant::now();
                if now >= at {
                    if let Some(expected) = reason.expected_state() {
                        let still_expected = {
                            let sessions = shared.sessions.lock().unwrap();
                            sessions
                                .get(&session_id)
                                .map(|s| s.state() == expected)
                                .unwrap_or(false)
                        };
                        if !still_expected {
                            return;
                        }
                    }
                    shared.close_inner(&session_id, reason);
                    return;
                }
                // 等到本次读取到的截止；到点后回到循环顶部重新读取（可能被刷新）。
                tokio::time::sleep_until(at).await;
            }
        });
    }

    fn replace_listener(&self, session_id: &str, handle: DataListenerHandle) {
        let mut handles = self.handles.lock().unwrap();
        match handles.get_mut(session_id) {
            Some(existing) => {
                if let Some(shutdown) = existing.shutdown.replace(handle.shutdown) {
                    shutdown();
                }
            }
            None => {
                handles.insert(
                    session_id.to_string(),
                    SessionHandle {
                        shutdown: Some(handle.shutdown),
                        deadline: Arc::new(Deadline::new()),
                    },
                );
            }
        }
    }
}

impl CloseReason {
    /// 定时器到点后的关闭原因所对应的「期望状态」；非定时原因返回 `None`。
    fn expected_state(self) -> Option<SessionState> {
        match self {
            CloseReason::AwaitingTimeout => Some(SessionState::Awaiting),
            CloseReason::GraceTimeout => Some(SessionState::Grace),
            _ => None,
        }
    }
}

fn map_param_error(err: ParamError) -> ManagerError {
    ManagerError::InvalidParams(err.to_string())
}

fn map_transport_error(err: TransportError) -> ManagerError {
    match err {
        TransportError::NotFound => ManagerError::PortNotFound,
        TransportError::PermissionDenied => ManagerError::PermissionDenied,
        TransportError::Busy => ManagerError::PortBusy,
        other => ManagerError::SerialOpenFailed(other.to_string()),
    }
}

/// 拼数据面 WebSocket URL：`ws://127.0.0.1:<port>/ws/<token>`。
fn data_url(port: u16, token: String) -> String {
    format!("ws://127.0.0.1:{port}/ws/{token}")
}
