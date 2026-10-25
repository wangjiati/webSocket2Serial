//! 集成测试共享替身。
//!
//! 本模块只被测试目标引用；`FakeTransport` 通过共享状态注入读数据、记录写数据，
//! 并可注入各类 `TransportError`。生产代码不得依赖这里的任何类型。

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;

use websocket2serial::config::SessionConfig;
use websocket2serial::protocol::{FlowControl, OpenParams, OpenResult, Parity};
use websocket2serial::serial::{SerialParams, SerialTransport, TransportError, TransportFactory};
use websocket2serial::session::{
    DataError, DataListenerFactory, DataListenerHandle, Session, SessionManager,
};

/// 假串口：多个线程通过同一个 `Arc<Mutex<FakeState>>` 观察/修改状态。
#[derive(Debug, Clone, Default)]
pub struct FakeTransport {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Debug, Default)]
struct FakeState {
    /// 待读取的数据块；每块按顺序交付，块内可被较小的 `buf` 拆开。
    rx: VecDeque<Vec<u8>>,
    written: Vec<u8>,
    next_read_error: Option<TransportError>,
    next_write_error: Option<TransportError>,
    dtr: Option<bool>,
    rts: Option<bool>,
    read_calls: usize,
    write_calls: usize,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一段待读取数据；连续调用保留块边界。
    pub fn push_rx(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.state.lock().unwrap().rx.push_back(bytes.to_vec());
    }

    /// 下一次 `read` 立即返回指定错误。
    pub fn fail_next_read(&self, error: TransportError) {
        self.state.lock().unwrap().next_read_error = Some(error);
    }

    /// 下一次 `write` 立即返回指定错误。
    pub fn fail_next_write(&self, error: TransportError) {
        self.state.lock().unwrap().next_write_error = Some(error);
    }

    /// 目前已写入串口的全部字节。
    pub fn written(&self) -> Vec<u8> {
        self.state.lock().unwrap().written.clone()
    }

    /// 取走并清空目前已写入的字节。
    pub fn take_written(&self) -> Vec<u8> {
        std::mem::take(&mut self.state.lock().unwrap().written)
    }

    pub fn dtr(&self) -> Option<bool> {
        self.state.lock().unwrap().dtr
    }

    pub fn rts(&self) -> Option<bool> {
        self.state.lock().unwrap().rts
    }

    pub fn read_calls(&self) -> usize {
        self.state.lock().unwrap().read_calls
    }

    pub fn write_calls(&self) -> usize {
        self.state.lock().unwrap().write_calls
    }

    /// 等待串口收到的写数据恰好等于 `expected`；超时则 panic 并带出实际值。
    pub async fn expect_tx(&self, expected: &[u8]) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let actual = self.written();
            if actual == expected {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expect_tx 超时：期望 {:?}，实际 {:?}",
                expected,
                actual
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl SerialTransport for FakeTransport {
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        let mut state = self.state.lock().unwrap();
        state.read_calls += 1;
        if let Some(error) = state.next_read_error.take() {
            return Err(error);
        }
        if let Some(mut chunk) = state.rx.pop_front() {
            let n = buf.len().min(chunk.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            if n < chunk.len() {
                chunk.drain(..n);
                state.rx.push_front(chunk);
            }
            return Ok(n);
        }
        drop(state);
        // 模拟真实驱动：无数据时阻塞到超时再返回 Timeout，避免读线程空转。
        std::thread::sleep(timeout.clamp(Duration::from_millis(1), Duration::from_millis(10)));
        Err(TransportError::Timeout)
    }

    fn write(&mut self, buf: &[u8], _timeout: Duration) -> Result<usize, TransportError> {
        let mut state = self.state.lock().unwrap();
        state.write_calls += 1;
        if let Some(error) = state.next_write_error.take() {
            return Err(error);
        }
        state.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError> {
        self.state.lock().unwrap().dtr = Some(level);
        Ok(())
    }

    fn set_rts(&mut self, level: bool) -> Result<(), TransportError> {
        self.state.lock().unwrap().rts = Some(level);
        Ok(())
    }
}

/// 假串口工厂：每次 `open` 生成新的 `FakeTransport`，并记录次数与参数。
#[derive(Debug, Default)]
pub struct FakeTransportFactory {
    state: Mutex<FactoryState>,
}

#[derive(Debug, Default)]
struct FactoryState {
    open_count: usize,
    next_open_error: Option<TransportError>,
    transports: Vec<Arc<FakeTransport>>,
    opens: Vec<SerialParams>,
    /// 若为 `Some`，下一次 `open` 会先发出 started 信号再阻塞，直到被 `release`。
    open_gate: Option<OpenGate>,
}

/// 阻塞下一次 `open` 的闸门：open 进入后发 started，随后阻塞等 release。
struct OpenGate {
    started: tokio::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

// `std::sync::mpsc::Receiver` 无 Debug；给 OpenGate 手动实现以保持
// `FakeTransportFactory: Debug`（FactoryState 派生 Debug 需要）。
impl std::fmt::Debug for OpenGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenGate").finish_non_exhaustive()
    }
}

/// 测试侧持有的闸门句柄：可异步等待 open 已进入、可放行。
pub struct OpenGateHandle {
    started: tokio::sync::mpsc::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
}

impl OpenGateHandle {
    /// 等待下一次 `open` 进入工厂（已发出 started）。异步等待，不阻塞运行时。
    pub async fn wait_started(&mut self) {
        self.started.recv().await.expect("open 尚未进入工厂");
    }

    /// 放行被阻塞的 `open`。
    pub fn release(self) {
        let _ = self.release.send(());
    }
}

impl FakeTransportFactory {
    pub fn new() -> Self {
        Self::default()
    }

    /// 下一次 `open` 返回指定错误（如 NotFound/PermissionDenied/Busy）。
    pub fn fail_next_open(&self, error: TransportError) {
        self.state.lock().unwrap().next_open_error = Some(error);
    }

    /// 让下一次 `open` 进入工厂后阻塞，返回闸门句柄供测试协调并发时序。
    pub fn gate_next_open(&self) -> OpenGateHandle {
        let (started_tx, started_rx) = tokio::sync::mpsc::channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        self.state.lock().unwrap().open_gate = Some(OpenGate {
            started: started_tx,
            release: release_rx,
        });
        OpenGateHandle {
            started: started_rx,
            release: release_tx,
        }
    }

    pub fn open_count(&self) -> usize {
        self.state.lock().unwrap().open_count
    }

    /// 最近一次成功打开的假串口；尚未打开时 panic。
    pub fn transport(&self) -> Arc<FakeTransport> {
        self.state
            .lock()
            .unwrap()
            .transports
            .last()
            .cloned()
            .expect("尚未打开任何 FakeTransport")
    }

    pub fn transport_at(&self, index: usize) -> Option<Arc<FakeTransport>> {
        self.state.lock().unwrap().transports.get(index).cloned()
    }

    /// 最近一次成功打开所用参数。
    pub fn last_params(&self) -> Option<SerialParams> {
        self.state.lock().unwrap().opens.last().cloned()
    }
}

impl TransportFactory for FakeTransportFactory {
    fn open(&self, params: &SerialParams) -> Result<Box<dyn SerialTransport>, TransportError> {
        let gate = {
            let mut state = self.state.lock().unwrap();
            state.open_count += 1;
            if let Some(error) = state.next_open_error.take() {
                return Err(error);
            }
            state.open_gate.take()
        };
        // 闸门阻塞放在锁外，避免占住 factory 锁。
        if let Some(gate) = gate {
            let _ = gate.started.try_send(());
            let _ = gate.release.recv();
        }
        let transport = FakeTransport::new();
        let mut state = self.state.lock().unwrap();
        state.transports.push(Arc::new(transport.clone()));
        state.opens.push(params.clone());
        Ok(Box::new(transport))
    }
}

/// 假数据监听器：`bind` 分配一个递增的临时端口，`shutdown` 无实际操作。
/// 用 `AtomicU16` 保证每次复用会话都会得到不同的端口（从而 URL 也不同）。
#[derive(Debug, Default)]
pub struct FakeDataListener {
    next_port: AtomicU16,
    bind_count: AtomicUsize,
}

impl FakeDataListener {
    pub fn new() -> Self {
        Self {
            next_port: AtomicU16::new(20_000),
            bind_count: AtomicUsize::new(0),
        }
    }

    pub fn bind_count(&self) -> usize {
        self.bind_count.load(Ordering::SeqCst)
    }
}

impl DataListenerFactory for FakeDataListener {
    fn bind(&self, _session: Arc<Session>) -> Result<DataListenerHandle, DataError> {
        self.bind_count.fetch_add(1, Ordering::SeqCst);
        let port = self.next_port.fetch_add(1, Ordering::SeqCst);
        Ok(DataListenerHandle {
            port,
            shutdown: Box::new(|| {}),
        })
    }
}

/// 构造一个带假串口/假数据监听器的 `SessionManager`（默认 `max_sessions`）。
pub fn test_manager() -> (SessionManager, Arc<FakeTransportFactory>) {
    test_manager_with_max_sessions(8)
}

/// 构造一个指定 `max_sessions` 的测试管理器。
pub fn test_manager_with_max_sessions(
    max_sessions: usize,
) -> (SessionManager, Arc<FakeTransportFactory>) {
    let cfg = SessionConfig {
        max_sessions,
        ..SessionConfig::default()
    };
    let factory = Arc::new(FakeTransportFactory::new());
    let (events, _) = broadcast::channel(16);
    let mgr = SessionManager::new(
        factory.clone(),
        Arc::new(FakeDataListener::new()),
        cfg,
        events,
    );
    (mgr, factory)
}

/// 生成一组可用的 `session.open` 参数。
pub fn sample_open(path: &str) -> OpenParams {
    OpenParams {
        path: path.to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        parity: Parity::None,
        stop_bits: 1,
        flow_control: FlowControl::None,
        dtr: None,
        rts: None,
    }
}

/// 从 `OpenResult.data_url` 里取回 token（URL 形如 `ws://127.0.0.1:<port>/ws/<token>`）。
pub fn token_of(result: &OpenResult) -> String {
    result
        .data_url
        .rsplit('/')
        .next()
        .expect("data_url 缺少 token")
        .to_string()
}
