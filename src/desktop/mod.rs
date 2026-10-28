//! Windows 原生管理窗口；其他平台保留无窗口模式。

pub mod state;

#[cfg(windows)]
mod windows;

use std::io;

#[derive(Debug)]
enum DesktopCommand {
    Show(String),
    Hide,
    Shutdown,
}

/// 主线程持有的桌面窗口控制器。
pub struct DesktopHandle {
    command_tx: std::sync::mpsc::Sender<DesktopCommand>,
    #[cfg(not(windows))]
    _command_rx: Option<std::sync::mpsc::Receiver<DesktopCommand>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl DesktopHandle {
    /// 启动 UI 线程。窗口创建与 WebView2 初始化失败只影响内嵌 UI。
    pub fn start(admin_url: String, visible: bool) -> io::Result<DesktopHandle> {
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        #[cfg(windows)]
        let join = {
            let ui_command_tx = command_tx.clone();
            Some(
                std::thread::Builder::new()
                    .name("ws2s-desktop".into())
                    .spawn(move || windows::run(admin_url, visible, ui_command_tx, command_rx))
                    .map_err(|err| io::Error::new(err.kind(), err))?,
            )
        };
        #[cfg(not(windows))]
        let join = {
            let _ = (admin_url, visible);
            None
        };

        Ok(DesktopHandle {
            command_tx,
            #[cfg(not(windows))]
            _command_rx: Some(command_rx),
            join,
        })
    }

    pub fn show(&self, admin_url: impl Into<String>) {
        let _ = self.command_tx.send(DesktopCommand::Show(admin_url.into()));
    }

    pub fn hide(&self) {
        let _ = self.command_tx.send(DesktopCommand::Hide);
    }

    pub fn shutdown(&mut self) {
        let _ = self.command_tx.send(DesktopCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for DesktopHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 第二实例请求已有实例显示窗口；找不到窗口时返回 false。
#[cfg(windows)]
pub fn wake_existing() -> bool {
    windows::wake_existing()
}

#[cfg(not(windows))]
pub fn wake_existing() -> bool {
    false
}
