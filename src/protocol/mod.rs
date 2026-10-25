//! 管理协议 DTO：请求/应答、7 个动作、5 个事件、错误码与 close code。

pub mod errors;
pub mod events;
pub mod management;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use errors::{CloseCode, ErrorBody, ErrorCode};
pub use events::{
    DebugFrame, DebugFrameData, Event, HelloData, PortsChangedData, RemovedPort, SessionStateData,
};
pub use management::{
    AdminSetParams, AdminState, CloseParams, CloseResult, Counters, DebugSubscribeParams,
    DebugSubscribeResult, GapInfo, OpenParams, OpenResult, PortInfo, PortsListResult, SessionInfo,
    SessionListResult, SessionParams, SessionSelector, WriteParams, WriteResult,
};

/// 管理请求：`{"id":"...","action":"...","params":{...}}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub action: ActionName,
    pub params: Value,
}

/// v1 的 7 个管理动作；未知动作落到 [`ActionName::Unknown`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionName {
    #[serde(rename = "ports.list")]
    PortsList,
    #[serde(rename = "session.open")]
    SessionOpen,
    #[serde(rename = "session.close")]
    SessionClose,
    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "session.write")]
    SessionWrite,
    #[serde(rename = "debug.subscribe")]
    DebugSubscribe,
    #[serde(rename = "admin.set")]
    AdminSet,
    /// 未知 action；上层据此返回 `UNKNOWN_ACTION`。
    #[serde(other)]
    Unknown,
}

/// 管理应答：成功时带 `data`，失败时带 `error`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

/// 会话状态；session 层直接复用，避免两套状态类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Awaiting,
    Active,
    Grace,
    Closed,
}
