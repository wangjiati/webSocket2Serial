//! 数据 WebSocket 监听器工厂与句柄。
//!
//! 每个会话在 `session.open` 时绑定一个监听 `127.0.0.1:0`（系统分配临时端口）
//! 的数据 WS 服务；会话关闭/复用时通过 [`DataListenerHandle::shutdown`] 关闭。
//! 真实实现（axum 服务）在任务 8 落地，这里只定义工厂 trait 与句柄。

use std::sync::Arc;

use super::session::Session;

/// 数据监听器绑定失败。
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("绑定数据监听器失败：{0}")]
    Bind(std::io::Error),
}

/// 数据监听器工厂：为会话绑定数据 WS 服务并返回句柄。
///
/// 实现必须把监听绑定到 `127.0.0.1:0`，并返回实际分配的端口。
pub trait DataListenerFactory: Send + Sync {
    fn bind(&self, session: Arc<Session>) -> Result<DataListenerHandle, DataError>;
}

/// 已绑定数据监听器的句柄；`shutdown` 调用一次即关闭服务。
pub struct DataListenerHandle {
    /// 实际绑定的端口（用于拼 `dataUrl`）。
    pub port: u16,
    /// 关闭该监听器；`FnOnce` 保证只执行一次。
    pub shutdown: Box<dyn FnOnce() + Send>,
}
