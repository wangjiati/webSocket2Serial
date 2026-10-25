//! 会话：状态机、token、计数与调试环形缓冲，以及会话管理器与数据监听器。

pub mod counters;
pub mod data_listener;
pub mod debug_ring;
mod io;
pub mod manager;
#[allow(clippy::module_inception)] // 计划指定文件名为 session/session.rs
pub mod session;

pub use crate::protocol::DebugFrame;
pub use counters::Counters;
pub use data_listener::{DataError, DataListenerFactory, DataListenerHandle};
pub use debug_ring::DebugRing;
pub use manager::{AttachError, ManagerError, SessionManager};
pub use session::{AttachedData, CloseReason, Session, SessionError};
