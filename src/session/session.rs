//! 会话：状态机、token、计数与调试环形缓冲。
//!
//! 本模块只负责会话的纯状态与计数；串口 I/O 线程由任务 7 接线，数据 WS
//! 与管理器由任务 6/8 实现。`close()` 只改状态并通过 close watch / `closed_tx`
//! 通知，不直接发送 `session.state` 管理事件（由管理器负责）。

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{broadcast, mpsc, watch};

use crate::config::SessionConfig;
use crate::protocol::{
    Counters, DebugFrame, ErrorBody, Event, GapInfo, SessionInfo, SessionParams, SessionState,
};
use crate::serial::SerialParams;

use super::debug_ring::DebugRing;

/// 调试环形缓冲的数据字节上限（64 KiB）。
const DEBUG_RING_BYTES: usize = 64 * 1024;

/// 会话关闭原因；决定数据面 WebSocket close code 与 `session.state` 的 reason。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// 网页主动正常关闭（1000）。
    ClientClosed,
    /// 管理端关闭（4004）。
    AdminClosed,
    /// 串口设备消失（4001）。
    DeviceLost,
    /// 写停滞超时（4007）。
    WriteTimeout,
    /// 串口读写 I/O 错误（4007）。
    SerialIo,
    /// 接收队列背压（4005）。
    Backpressure,
    /// 宽限超时；数据 WS 已断开，无 close code。
    GraceTimeout,
    /// awaiting 连接超时；数据 WS 尚未建立，无 close code。
    AwaitingTimeout,
}

impl CloseReason {
    /// 数据面 WebSocket close code；无数据连接的场景返回 `None`。
    pub fn close_code(self) -> Option<u16> {
        match self {
            CloseReason::ClientClosed => Some(1000),
            CloseReason::AdminClosed => Some(4004),
            CloseReason::DeviceLost => Some(4001),
            CloseReason::WriteTimeout | CloseReason::SerialIo => Some(4007),
            CloseReason::Backpressure => Some(4005),
            CloseReason::GraceTimeout | CloseReason::AwaitingTimeout => None,
        }
    }

    /// `session.state` 事件里的 reason 字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            CloseReason::ClientClosed => "CLIENT_CLOSED",
            CloseReason::AdminClosed => "ADMIN_CLOSED",
            CloseReason::DeviceLost => "DEVICE_LOST",
            CloseReason::WriteTimeout => "WRITE_TIMEOUT",
            CloseReason::SerialIo => "SERIAL_IO_ERROR",
            CloseReason::Backpressure => "BACKPRESSURE",
            CloseReason::GraceTimeout => "GRACE_TIMEOUT",
            CloseReason::AwaitingTimeout => "SESSION_EXPIRED",
        }
    }
}

/// 状态迁移失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("会话已处于 active，无法重复附加")]
    AlreadyActive,
    #[error("会话当前非 active，无法进入宽限")]
    NotActive,
    #[error("会话当前非 grace，无法恢复")]
    NotGrace,
    #[error("会话已关闭")]
    Closed,
}

/// 数据 WS 附加会话时拿到的收发通道。
#[derive(Debug)]
pub struct AttachedData {
    /// 串口 → 网页的数据块接收端。
    pub rx: mpsc::Receiver<Vec<u8>>,
    /// 网页 → 串口的数据块发送端。
    pub tx: mpsc::Sender<Vec<u8>>,
}

/// 可变会话状态；用 `Mutex` 保护以便 `Arc<Session>` 下的状态迁移与并发计数。
struct SessionInner {
    state: SessionState,
    token: String,
    counters: Counters,
    connect_by: Option<u64>,
    active_since: Option<u64>,
    grace_ends_at: Option<u64>,
    /// 本次宽限的开始时间与起始丢弃字节，供 `resume` 计算 `last_gap`。
    grace_started_at: Option<u64>,
    grace_discarded_start: u64,
    last_gap: Option<GapInfo>,
    last_error: Option<ErrorBody>,
    last_rx_at: Option<u64>,
    last_tx_at: Option<u64>,
}

/// 单个串口会话。
pub struct Session {
    id: String,
    params: SerialParams,
    cfg: SessionConfig,
    created_at: u64,
    data_port: AtomicU16,
    /// 任务 7 的读/写线程向 `events` 发 `debug.frame`；本任务尚未接线。
    #[allow(dead_code)]
    events: broadcast::Sender<Event>,
    closed_tx: mpsc::UnboundedSender<String>,
    close_tx: watch::Sender<Option<CloseReason>>,
    close_rx: watch::Receiver<Option<CloseReason>>,
    inner: Mutex<SessionInner>,
    debug_ring: Mutex<DebugRing>,
    debug_seq: AtomicU64,
    /// 串口 → 网页的数据块接收端（数据 WS 在 attach 时取走、宽限时归还）。
    rx: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    /// 网页 → 串口的数据块发送端（数据 WS 每次 attach 克隆一份）。
    tx: mpsc::Sender<Vec<u8>>,
}

impl Session {
    /// 创建 awaiting 会话。
    pub fn new(
        id: impl Into<String>,
        params: SerialParams,
        cfg: SessionConfig,
        events: broadcast::Sender<Event>,
        closed_tx: mpsc::UnboundedSender<String>,
    ) -> Self {
        let id = id.into();
        let created_at = now_ms();
        let connect_by = created_at + cfg.connect_timeout_ms;
        let rx_capacity = (cfg.rx_queue_bytes / 4096).max(1);
        // rx/tx 的串口侧端点（发送端/接收端）由任务 7 的读写线程持有；
        // 本任务只定义数据 WS 一侧的通道接口。
        let (_rx_tx, rx) = mpsc::channel::<Vec<u8>>(rx_capacity);
        let (tx, _tx_rx) = mpsc::channel::<Vec<u8>>(1);
        let (close_tx, close_rx) = watch::channel(None);
        Self {
            id,
            params,
            cfg,
            created_at,
            data_port: AtomicU16::new(0),
            events,
            closed_tx,
            close_tx,
            close_rx,
            inner: Mutex::new(SessionInner {
                state: SessionState::Awaiting,
                token: generate_token(),
                counters: Counters::default(),
                connect_by: Some(connect_by),
                active_since: None,
                grace_ends_at: None,
                grace_started_at: None,
                grace_discarded_start: 0,
                last_gap: None,
                last_error: None,
                last_rx_at: None,
                last_tx_at: None,
            }),
            debug_ring: Mutex::new(DebugRing::new(DEBUG_RING_BYTES)),
            debug_seq: AtomicU64::new(1),
            rx: Mutex::new(Some(rx)),
            tx,
        }
    }

    /// 会话 ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 当前状态。
    pub fn state(&self) -> SessionState {
        self.inner.lock().unwrap().state
    }

    /// 当前 token（128 位随机值的十六进制）。
    pub fn current_token(&self) -> String {
        self.inner.lock().unwrap().token.clone()
    }

    /// 轮换 token，返回新 token；旧 token 立即失效。
    pub fn rotate_token(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        inner.token = generate_token();
        inner.token.clone()
    }

    /// 校验 token 是否与当前一致。
    pub fn check_token(&self, token: &str) -> bool {
        self.inner.lock().unwrap().token == token
    }

    /// 当前计数快照（拷贝）。
    pub fn counters(&self) -> Counters {
        self.inner.lock().unwrap().counters
    }

    /// 记录从串口读到的字节数（active 阶段交付），并刷新 `lastRxAt`。
    pub fn record_rx(&self, bytes: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.counters.add_rx(bytes);
        inner.last_rx_at = Some(now_ms());
    }

    /// 记录写入串口的字节数，并刷新 `lastTxAt`。
    pub fn record_tx(&self, bytes: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.counters.add_tx(bytes);
        inner.last_tx_at = Some(now_ms());
    }

    /// 记录宽限期内读到但未交付的字节数（同时计入 `rxBytes`，维持不变式）。
    pub fn record_discarded(&self, bytes: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.counters.add_rx(bytes);
        inner.counters.add_discarded(bytes);
        inner.last_rx_at = Some(now_ms());
    }

    /// 取走数据 WS 接收端；宽限归还前再次取走会得到 `None`。
    pub fn take_rx(&self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.rx.lock().unwrap().take()
    }

    /// 数据 WS 异常断开时归还接收端。
    pub fn return_rx(&self, rx: mpsc::Receiver<Vec<u8>>) {
        *self.rx.lock().unwrap() = Some(rx);
    }

    /// 绑定数据端口（由管理器在数据监听器 bind 后调用）。
    pub fn set_data_port(&self, port: u16) {
        self.data_port.store(port, Ordering::Relaxed);
    }

    /// 数据端口；尚未绑定时为 0。
    pub fn data_port(&self) -> u16 {
        self.data_port.load(Ordering::Relaxed)
    }

    /// 数据 WS 附加：`awaiting`/`grace` → `active`，返回收发通道。
    pub fn attach(&self) -> Result<AttachedData, SessionError> {
        {
            let mut inner = self.inner.lock().unwrap();
            match inner.state {
                SessionState::Awaiting => {
                    inner.state = SessionState::Active;
                    inner.active_since = Some(now_ms());
                }
                SessionState::Grace => resume_locked(&mut inner),
                SessionState::Active => return Err(SessionError::AlreadyActive),
                SessionState::Closed => return Err(SessionError::Closed),
            }
        }
        let rx = self.take_rx().unwrap_or_else(closed_rx);
        Ok(AttachedData {
            rx,
            tx: self.tx.clone(),
        })
    }

    /// 进入宽限期：`active` → `grace`，记录宽限起止信息。
    pub fn enter_grace(&self) -> Result<(), SessionError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.state {
            SessionState::Active => {
                inner.state = SessionState::Grace;
                let now = now_ms();
                inner.active_since = None;
                inner.grace_ends_at = Some(now + self.cfg.grace_ms);
                inner.grace_started_at = Some(now);
                inner.grace_discarded_start = inner.counters.rx_discarded_bytes;
                Ok(())
            }
            SessionState::Closed => Err(SessionError::Closed),
            _ => Err(SessionError::NotActive),
        }
    }

    /// 从宽限期恢复：`grace` → `active`，写入 `lastGap`。
    pub fn resume(&self) -> Result<(), SessionError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.state {
            SessionState::Grace => {
                resume_locked(&mut inner);
                Ok(())
            }
            SessionState::Closed => Err(SessionError::Closed),
            _ => Err(SessionError::NotGrace),
        }
    }

    /// 关闭会话（幂等）：改状态并通过 close watch 与 `closed_tx` 通知；
    /// 不直接发管理事件（由管理器负责）。
    pub fn close(&self, reason: CloseReason) {
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == SessionState::Closed {
                return;
            }
            inner.state = SessionState::Closed;
            inner.active_since = None;
            inner.grace_ends_at = None;
        }
        let _ = self.close_tx.send(Some(reason));
        let _ = self.closed_tx.send(self.id.clone());
    }

    /// 订阅关闭通知（数据 WS 据此发送 close code 并退出）。
    pub fn close_subscribe(&self) -> watch::Receiver<Option<CloseReason>> {
        self.close_rx.clone()
    }

    /// 分配下一个调试帧序号（从 1 递增）。
    pub fn next_debug_seq(&self) -> u64 {
        self.debug_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// 追加一条调试帧到环形缓冲。
    pub fn push_debug_frame(&self, frame: DebugFrame) {
        self.debug_ring.lock().unwrap().push(frame);
    }

    /// 回放当前调试环形缓冲（按插入顺序）。
    pub fn debug_snapshot(&self) -> Vec<DebugFrame> {
        self.debug_ring.lock().unwrap().snapshot()
    }

    /// 生成 `session.list` 用的会话信息快照。
    pub fn info(&self) -> SessionInfo {
        let inner = self.inner.lock().unwrap();
        SessionInfo {
            session_id: self.id.clone(),
            state: inner.state,
            port: self.params.path.clone(),
            params: SessionParams {
                baud_rate: self.params.baud_rate,
                data_bits: self.params.data_bits,
                parity: self.params.parity,
                stop_bits: self.params.stop_bits,
                flow_control: self.params.flow_control,
            },
            created_at: self.created_at,
            data_port: self.data_port.load(Ordering::Relaxed),
            connect_by: inner.connect_by,
            active_since: inner.active_since,
            grace_ends_at: inner.grace_ends_at,
            counters: inner.counters,
            last_gap: inner.last_gap.clone(),
            last_error: inner.last_error.clone(),
            last_rx_at: inner.last_rx_at,
            last_tx_at: inner.last_tx_at,
        }
    }
}

/// 从宽限期恢复到 active（需已持锁）。
fn resume_locked(inner: &mut SessionInner) {
    let now = now_ms();
    inner.state = SessionState::Active;
    inner.active_since = Some(now);
    inner.grace_ends_at = None;
    if let Some(started) = inner.grace_started_at.take() {
        let discarded = inner
            .counters
            .rx_discarded_bytes
            .saturating_sub(inner.grace_discarded_start);
        inner.last_gap = Some(GapInfo {
            started_at: started,
            duration_ms: now.saturating_sub(started),
            discarded_bytes: discarded,
            resumed: true,
        });
    }
}

/// 生成 128 位随机值的十六进制 token。
fn generate_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 当前 Unix 时间戳（毫秒）。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 已关闭的空接收端；用于防御性回退（正常情况下 attach 总能取到 rx）。
fn closed_rx() -> mpsc::Receiver<Vec<u8>> {
    let (_tx, rx) = mpsc::channel::<Vec<u8>>(1);
    rx
}
