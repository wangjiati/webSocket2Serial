//! 管理协议的错误码、错误体与数据面 close code。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 管理协议错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// JSON 格式或请求结构错误。
    InvalidRequest,
    /// 未知 action。
    UnknownAction,
    /// 参数缺失或取值非法。
    InvalidParams,
    /// 指定串口不存在。
    PortNotFound,
    /// 串口已被占用，或同端口已有 active 会话。
    PortBusy,
    /// 无权限打开串口。
    PermissionDenied,
    /// 其他打开失败。
    SerialOpenFailed,
    /// 会话不存在或已关闭。
    SessionNotFound,
    /// 当前状态不允许该操作。
    SessionState,
    /// 超过最大会话数或队列上限。
    LimitExceeded,
    /// 未归类的内部错误。
    Internal,
}

/// 错误应答中的 `error` 对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl ErrorBody {
    /// 构造不带 `detail` 的错误体。
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    /// 附加平台原始错误码与文本。
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// 数据面 WebSocket close code。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CloseCode {
    Normal = 1000,
    GoingAway = 1001,
    UnsupportedData = 1003,
    MessageTooBig = 1009,
    DeviceLost = 4001,
    PortBusy = 4002,
    AdminClosed = 4004,
    Backpressure = 4005,
    SerialIoError = 4007,
}

impl CloseCode {
    /// close code 的数值形式，用于写入 WebSocket Close 帧。
    pub fn as_u16(self) -> u16 {
        self as u16
    }
}
