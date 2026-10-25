//! 应用状态聚合与启动。
//!
//! [`AppState`] 是管理 WS、HTTP 管理页与托盘共享的运行时全局状态；
//! [`AdminController`] 抽象管理页服务开关（任务 12 实现真实版，测试用 fake）；
//! [`build_app`] 绑定管理端口并启动管理 WS 服务。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{broadcast, oneshot};

use crate::config::Config;
use crate::hotplug::{Hotplug, POLL_INTERVAL};
use crate::protocol::{AdminState, Event};
use crate::serial::PortScanner;
use crate::session::SessionManager;

/// 运行时全局状态。
pub struct AppState {
    pub config: Config,
    pub manager: Arc<SessionManager>,
    pub admin: Arc<dyn AdminController>,
    pub scanner: Arc<dyn PortScanner>,
    pub events: broadcast::Sender<Event>,
    pub hotplug: Arc<Hotplug>,
    pub started_at: u64,
}

/// 管理页服务控制。
///
/// 对象安全：无泛型、无 `Self` 返回。`set_enabled` 返回对象安全的 boxed `Future`，
/// disable 必须 await 到 serve 任务退出、端口真正释放后才返回，避免关→立即开的
/// `AddrInUse`；真实实现见 [`crate::http::AdminService`]，测试用 fake 见
/// `tests/common` 的 `FakeAdminController`。
pub trait AdminController: Send + Sync {
    fn status(&self) -> AdminState;
    /// 当前开机自启状态（单一真值源，供 `hello.autostart` 上报）。
    fn autostart(&self) -> bool;
    fn set_enabled(&self, enabled: bool) -> Pin<Box<dyn Future<Output = AdminState> + Send + '_>>;
}

/// 已启动的应用：持有 [`AppState`] 与管理 WS 的 shutdown 信号。
pub struct App {
    pub state: Arc<AppState>,
    pub management_port: u16,
    shutdown: Option<oneshot::Sender<()>>,
}

impl App {
    /// 停止管理 WS accept 循环；已接受的连接独立处理至完成。
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// 绑定管理端口并启动管理 WS 服务。
///
/// `manager` 由调用方以 `Arc::new_cyclic` 构造（数据监听器需其弱引用）。
/// `config.server.management_port` 为 `0` 时取系统分配的临时端口，并回写实际
/// 端口供 `hello` 与 `/api/config` 上报。HTTP 管理页由任务 12 接入。
pub fn build_app(
    config: Config,
    manager: Arc<SessionManager>,
    admin: Arc<dyn AdminController>,
    scanner: Arc<dyn PortScanner>,
    events: broadcast::Sender<Event>,
) -> std::io::Result<App> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", config.server.management_port))?;
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let management_port = listener.local_addr()?.port();

    let mut config = config;
    config.server.management_port = management_port;

    let started_at = now_ms();
    let hotplug = Arc::new(Hotplug::new(scanner.clone(), events.clone()));
    let state = Arc::new(AppState {
        config,
        manager,
        admin,
        scanner,
        events,
        hotplug: Arc::clone(&hotplug),
        started_at,
    });

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            crate::ws::management::serve(state, listener, shutdown_rx).await;
        });
    }
    // 热插拔轮询：启动后立即扫描一次，此后每 2s 一次。
    tokio::spawn(async move {
        hotplug.run(POLL_INTERVAL).await;
    });

    Ok(App {
        state,
        management_port,
        shutdown: Some(shutdown_tx),
    })
}

/// 当前 Unix 时间戳（毫秒）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
