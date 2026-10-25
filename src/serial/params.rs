//! 串口参数与校验。

use thiserror::Error;

use crate::protocol::{FlowControl, OpenParams, Parity};

/// 打开串口所需的完整参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialParams {
    pub path: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow_control: FlowControl,
    /// 打开后立即应用的 DTR 电平；`None` 表示沿用驱动默认。
    pub dtr: Option<bool>,
    /// 打开后立即应用的 RTS 电平；`None` 表示沿用驱动默认。
    pub rts: Option<bool>,
}

impl SerialParams {
    /// 校验参数是否可交给驱动打开。
    ///
    /// v1 只支持 5–8 数据位与 1/2 停止位（不支持 1.5）；parity/flow control
    /// 使用枚举，非法值在反序列化阶段已被拒绝。
    pub fn validate(&self) -> Result<(), ParamError> {
        if self.path.trim().is_empty() {
            return Err(ParamError::EmptyPath);
        }
        if self.baud_rate == 0 {
            return Err(ParamError::InvalidBaudRate(self.baud_rate));
        }
        if !(5..=8).contains(&self.data_bits) {
            return Err(ParamError::InvalidDataBits(self.data_bits));
        }
        if self.stop_bits != 1 && self.stop_bits != 2 {
            return Err(ParamError::InvalidStopBits(self.stop_bits));
        }
        // v1 只支持 none/odd/even；mark/space 在协议层保留，但底层 serialport 无法表达，
        // 在参数校验阶段直接拒绝为参数错误（任务 6 映射为 INVALID_PARAMS）。
        if !matches!(self.parity, Parity::None | Parity::Odd | Parity::Even) {
            return Err(ParamError::UnsupportedParity(self.parity));
        }
        Ok(())
    }
}

impl From<OpenParams> for SerialParams {
    fn from(params: OpenParams) -> Self {
        Self {
            path: params.path,
            baud_rate: params.baud_rate,
            data_bits: params.data_bits,
            parity: params.parity,
            stop_bits: params.stop_bits,
            flow_control: params.flow_control,
            dtr: params.dtr,
            rts: params.rts,
        }
    }
}

/// 串口参数校验错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParamError {
    #[error("串口路径不能为空")]
    EmptyPath,
    #[error("波特率必须大于 0，实际为 {0}")]
    InvalidBaudRate(u32),
    #[error("数据位必须是 5–8，实际为 {0}")]
    InvalidDataBits(u8),
    #[error("停止位必须是 1 或 2，实际为 {0}")]
    InvalidStopBits(u8),
    #[error("校验位 v1 仅支持 none/odd/even，实际为 {0:?}")]
    UnsupportedParity(Parity),
}
