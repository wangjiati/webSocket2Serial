//! 会话：状态机、token、计数与调试环形缓冲。

pub mod counters;
pub mod debug_ring;
#[allow(clippy::module_inception)] // 计划指定文件名为 session/session.rs
pub mod session;

pub use crate::protocol::DebugFrame;
pub use counters::Counters;
pub use debug_ring::DebugRing;
pub use session::{AttachedData, CloseReason, Session, SessionError};
