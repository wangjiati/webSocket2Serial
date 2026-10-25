//! 5 个管理事件的数据类型。

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::errors::ErrorBody;
use super::management::{AdminState, Counters, GapInfo, PortInfo, SessionParams};
use super::SessionState;

/// 服务端推送事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum Event {
    /// 管理连接建立。
    Hello(HelloData),
    /// 热插拔轮询发现串口变化。
    PortsChanged(PortsChangedData),
    /// 会话状态迁移。
    SessionState(SessionStateData),
    /// 已订阅会话的收发调试帧。
    DebugFrame(DebugFrameData),
    /// 管理页服务开关变化。
    AdminState(AdminState),
}

/// `hello` 事件数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloData {
    pub protocol: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub management_port: u16,
    pub admin: AdminState,
    pub autostart: bool,
}

/// `ports.changed` 事件数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortsChangedData {
    pub added: Vec<PortInfo>,
    pub removed: Vec<RemovedPort>,
}

/// `ports.changed.removed` 的元素；只带路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedPort {
    pub path: String,
}

/// `session.state` 事件数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateData {
    pub session_id: String,
    pub state: SessionState,
    pub port: String,
    pub params: SessionParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace_ends_at: Option<u64>,
    pub counters: Counters,
    pub last_gap: Option<GapInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ErrorBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rx_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tx_at: Option<u64>,
}

/// 单条调试帧；调试环形缓冲按会话保存该结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugFrame {
    pub direction: String,
    pub seq: u64,
    pub ts_ms: u64,
    pub size: usize,
    pub data_b64: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub replay: bool,
}

impl DebugFrame {
    /// 用原始字节构造一帧，`dataB64` 与 `size` 自动计算。
    pub fn new(direction: impl Into<String>, seq: u64, ts_ms: u64, data: Vec<u8>) -> Self {
        Self {
            direction: direction.into(),
            seq,
            ts_ms,
            size: data.len(),
            data_b64: STANDARD.encode(data),
            replay: false,
        }
    }

    /// 标记为回放帧。
    pub fn with_replay(mut self, replay: bool) -> Self {
        self.replay = replay;
        self
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// `debug.frame` 事件数据：会话 ID + 帧内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugFrameData {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(flatten)]
    pub frame: DebugFrame,
}
