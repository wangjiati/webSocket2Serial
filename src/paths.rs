//! 平台相关的配置 / 日志 / 锁文件路径。

use std::path::PathBuf;

/// 配置目录。
///
/// - Windows：`%APPDATA%\WebSocket2Serial`
/// - Linux：`$XDG_CONFIG_HOME/websocket2serial`（缺省 `~/.config/websocket2serial`）
pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        dirs::config_dir()
            .map(|d| d.join("WebSocket2Serial"))
            .unwrap_or_else(fallback)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map(|d| d.join("websocket2serial"))
            .or_else(|| dirs::config_dir().map(|d| d.join("websocket2serial")))
            .unwrap_or_else(fallback)
    }
}

/// 日志目录。
///
/// - Windows：`%LOCALAPPDATA%\WebSocket2Serial\logs`
/// - Linux：`~/.local/state/websocket2serial/logs`
pub fn log_dir() -> PathBuf {
    if cfg!(windows) {
        dirs::data_local_dir()
            .map(|d| d.join("WebSocket2Serial").join("logs"))
            .unwrap_or_else(fallback)
    } else {
        dirs::state_dir()
            .map(|d| d.join("websocket2serial").join("logs"))
            .or_else(|| {
                dirs::home_dir().map(|d| {
                    d.join(".local")
                        .join("state")
                        .join("websocket2serial")
                        .join("logs")
                })
            })
            .unwrap_or_else(fallback)
    }
}

/// 单实例锁文件路径。
///
/// - Linux：`$XDG_RUNTIME_DIR/websocket2serial.lock`（缺省退回配置目录）。
/// - Windows：单实例走命名互斥量，此处返回一个备用路径（不参与生产逻辑）。
pub fn lock_path() -> PathBuf {
    if cfg!(windows) {
        config_dir().join("websocket2serial.lock")
    } else {
        dirs::runtime_dir()
            .map(|d| d.join("websocket2serial.lock"))
            .unwrap_or_else(|| config_dir().join("websocket2serial.lock"))
    }
}

/// 平台目录缺失时的兜底：当前工作目录。
fn fallback() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
