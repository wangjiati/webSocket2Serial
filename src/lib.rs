//! WebSocket2Serial：WebSocket↔串口 本机常驻透传工具。
//!
//! 单 crate 单二进制工程。模块划分见设计文档 §11.2。

pub mod app;
pub mod cli;
pub mod config;
pub mod hotplug;
pub mod logging;
pub mod paths;
pub mod protocol;
pub mod serial;
pub mod session;
pub mod ws;

pub use app::{AdminController, AppState};
pub use cli::Cli;
pub use config::{Config, OriginAllowlist};
