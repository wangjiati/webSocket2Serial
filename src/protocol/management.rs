//! 7 个管理动作的参数与返回类型。

use serde::{Deserialize, Deserializer, Serialize};

use super::errors::ErrorBody;
use super::SessionState;

/// `session.open` 参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenParams {
    pub path: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: String,
    pub stop_bits: u8,
    pub flow_control: String,
    /// 打开后立即应用的 DTR 电平；缺省为驱动默认。
    #[serde(default)]
    pub dtr: Option<bool>,
    /// 打开后立即应用的 RTS 电平；缺省为驱动默认。
    #[serde(default)]
    pub rts: Option<bool>,
}

/// `session.open` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub session_id: String,
    pub data_url: String,
    pub connect_timeout_ms: u64,
    pub reused: bool,
    pub counters: Counters,
}

/// `session.close` 参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseParams {
    pub session_id: String,
}

/// `session.close` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseResult {
    pub closed: bool,
}

/// `session.list` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<SessionInfo>,
}

/// `session.write` 参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteParams {
    pub session_id: String,
    pub data_b64: String,
}

/// `session.write` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteResult {
    pub written: usize,
}

/// `debug.subscribe` 的会话选择器：`"all"` 或会话 ID 数组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSelector {
    All,
    Ids(Vec<String>),
}

impl<'de> Deserialize<'de> for SessionSelector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SelectorVisitor;

        impl<'de> serde::de::Visitor<'de> for SelectorVisitor {
            type Value = SessionSelector;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("\"all\" or an array of session ids")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if value == "all" {
                    Ok(SessionSelector::All)
                } else {
                    Err(E::invalid_value(
                        serde::de::Unexpected::Str(value),
                        &"\"all\" or an array of session ids",
                    ))
                }
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut ids = Vec::new();
                while let Some(id) = seq.next_element::<String>()? {
                    ids.push(id);
                }
                Ok(SessionSelector::Ids(ids))
            }
        }

        deserializer.deserialize_any(SelectorVisitor)
    }
}

impl Serialize for SessionSelector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            SessionSelector::All => serializer.serialize_str("all"),
            SessionSelector::Ids(ids) => ids.serialize(serializer),
        }
    }
}

/// `debug.subscribe` 参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugSubscribeParams {
    pub session_ids: SessionSelector,
    /// 是否回放会话内存环形缓冲，缺省为 `false`。
    #[serde(default)]
    pub replay: bool,
}

/// `debug.subscribe` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugSubscribeResult {
    pub subscribed: Vec<String>,
}

/// `admin.set` 参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminSetParams {
    pub enabled: bool,
}

/// 管理页服务状态；同时用于 `hello` 事件与 `admin.set` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminState {
    pub enabled: bool,
    pub url: String,
}

/// `ports.list` 返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortsListResult {
    pub ports: Vec<PortInfo>,
}

/// 串口快照；平台拿不到的字段为 `null`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    pub path: String,
    pub port_type: Option<String>,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

/// 会话参数快照（与 `OpenParams` 相同，但不含 `path`/`dtr`/`rts`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionParams {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: String,
    pub stop_bits: u8,
    pub flow_control: String,
}

/// `session.list` 中的会话信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    pub state: SessionState,
    pub port: String,
    pub params: SessionParams,
    pub created_at: u64,
    pub data_port: u16,
    pub connect_by: Option<u64>,
    pub active_since: Option<u64>,
    pub grace_ends_at: Option<u64>,
    pub counters: Counters,
    pub last_gap: Option<GapInfo>,
    pub last_error: Option<ErrorBody>,
    pub last_rx_at: Option<u64>,
    pub last_tx_at: Option<u64>,
}

/// 会话 64 位计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_discarded_bytes: u64,
}

impl Counters {
    /// 累计从串口读到的字节数（含宽限期丢弃）。
    pub fn add_rx(&mut self, bytes: u64) {
        self.rx_bytes = self.rx_bytes.saturating_add(bytes);
    }

    /// 累计写入串口的字节数。
    pub fn add_tx(&mut self, bytes: u64) {
        self.tx_bytes = self.tx_bytes.saturating_add(bytes);
    }

    /// 累计宽限期内读到但未交付的字节数。
    pub fn add_discarded(&mut self, bytes: u64) {
        self.rx_discarded_bytes = self.rx_discarded_bytes.saturating_add(bytes);
    }

    /// 已交付字节数：`rxBytes - rxDiscardedBytes`。
    pub fn delivered(&self) -> u64 {
        self.rx_bytes.saturating_sub(self.rx_discarded_bytes)
    }
}

/// 最近一次断线宽限信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GapInfo {
    pub started_at: u64,
    pub duration_ms: u64,
    pub discarded_bytes: u64,
    pub resumed: bool,
}
