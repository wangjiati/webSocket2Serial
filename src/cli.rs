//! 命令行参数。

use std::path::PathBuf;

use clap::Parser;

/// 命令行参数。
#[derive(Debug, Parser)]
#[command(name = "ws2serial", version, about = "WebSocket↔串口 本机常驻透传工具")]
pub struct Cli {
    /// 配置文件路径（缺省为平台默认路径）。
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// 强制本次启动开启管理页（覆盖配置中的 `admin.enabled`）。
    #[arg(long = "admin")]
    pub force_admin: bool,

    /// 启动时仅驻留托盘，不显示管理窗口（Windows）。
    #[arg(long)]
    pub minimized: bool,
}
