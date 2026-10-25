//! 会话 64 位计数。
//!
//! 类型与实现（`add_rx`/`add_tx`/`add_discarded`/`delivered`）已在
//! [`crate::protocol::Counters`]，这里只做 re-export，供 session 层统一引用。

pub use crate::protocol::Counters;
