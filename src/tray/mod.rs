//! Windows 托盘：状态、动作与线程句柄。
//!
//! - [`TrayAction`]：托盘菜单动作，由托盘线程经 [`tokio::sync::mpsc`] 发给 main。
//! - [`TrayUpdate`]：main 经 [`std::sync::mpsc`] 发给托盘线程的状态/气泡/关闭。
//! - [`TrayHandle`]：main 持有，用于发更新、收动作、退出时关闭。
//!
//! Linux 默认不编译托盘线程（可选 feature 未启用），以 headless 运行：
//! 动作通道永不触发，管理页与协议功能不受影响。

pub mod state;

pub use state::TrayStatus;

#[cfg(windows)]
mod windows;

/// 托盘菜单动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// 打开管理页（若已禁用则先启用）。
    OpenAdmin,
    /// 在系统浏览器中打开管理页（若已禁用则先启用）。
    OpenBrowser,
    /// 管理服务开关。
    SetAdmin(bool),
    /// 开机自启开关。
    SetAutostart(bool),
    /// 退出进程。
    Quit,
}

/// main 发给托盘线程的更新。
pub enum TrayUpdate {
    /// 更新图标 / tooltip / 菜单开关状态。
    State(TrayState),
    /// 弹出错误气泡。
    Bubble(String),
    /// 关闭托盘并退出消息循环。
    Shutdown,
}

/// 托盘当前状态快照。
#[derive(Debug, Clone)]
pub struct TrayState {
    pub status: TrayStatus,
    pub tooltip: String,
    pub admin_enabled: bool,
    pub autostart: bool,
}

/// 托盘句柄。
pub struct TrayHandle {
    update_tx: std::sync::mpsc::Sender<TrayUpdate>,
    /// 保持动作通道发送端存活：无托盘（headless）时 `action_rx` 永不关闭。
    _action_tx: tokio::sync::mpsc::UnboundedSender<TrayAction>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl TrayHandle {
    /// 启动托盘（若无平台托盘则为 headless），返回句柄与动作接收端。
    pub fn start(
        version: String,
        initial: TrayState,
    ) -> (TrayHandle, tokio::sync::mpsc::UnboundedReceiver<TrayAction>) {
        let (action_tx, action_rx) = tokio::sync::mpsc::unbounded_channel();
        let (update_tx, update_rx) = std::sync::mpsc::channel();

        #[cfg(windows)]
        let join = match windows::spawn(version, initial, action_tx.clone(), update_rx) {
            Ok(join) => Some(join),
            Err(err) => {
                tracing::warn!(error = %err, "托盘线程启动失败，以无托盘模式运行");
                None
            }
        };
        #[cfg(not(windows))]
        let join = {
            let _ = (version, initial, update_rx);
            None
        };

        (
            TrayHandle {
                update_tx,
                _action_tx: action_tx,
                join,
            },
            action_rx,
        )
    }

    /// 供原生窗口把第二实例唤醒统一转交主事件循环。
    pub fn action_sender(&self) -> tokio::sync::mpsc::UnboundedSender<TrayAction> {
        self._action_tx.clone()
    }

    /// 发送状态快照；托盘线程已退出时静默忽略。
    pub fn send_state(&self, state: TrayState) {
        let _ = self.update_tx.send(TrayUpdate::State(state));
    }

    /// 弹出错误气泡；托盘线程已退出时静默忽略。
    pub fn bubble(&self, message: impl Into<String>) {
        let _ = self.update_tx.send(TrayUpdate::Bubble(message.into()));
    }

    /// 关闭托盘并等待线程退出（幂等）。
    pub fn shutdown(&mut self) {
        let _ = self.update_tx.send(TrayUpdate::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
