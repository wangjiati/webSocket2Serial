//! 串口传输抽象：会话 I/O 只依赖本模块的 trait，真实实现见 `real`。

use std::time::Duration;

use thiserror::Error;

use super::params::SerialParams;

/// 串口读写与调制解调器控制线操作。
///
/// 实现必须能在独立阻塞线程中使用；`timeout` 是单次调用的上限（读超时或
/// 写停滞超时），由调用方按配置传入。
pub trait SerialTransport: Send {
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>;
    fn write(&mut self, buf: &[u8], timeout: Duration) -> Result<usize, TransportError>;
    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError>;
    fn set_rts(&mut self, level: bool) -> Result<(), TransportError>;
}

/// 打开串口并返回传输对象。
///
/// `open` 允许在调用线程阻塞；上层负责放到 `spawn_blocking` 中执行。
pub trait TransportFactory: Send + Sync {
    fn open(&self, params: &SerialParams) -> Result<Box<dyn SerialTransport>, TransportError>;
}

/// 串口传输错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransportError {
    /// 读超时：在本次超时窗口内没有数据。
    #[error("读串口超时")]
    Timeout,
    /// 设备已断开或句柄失效。
    #[error("串口设备已断开")]
    DeviceLost,
    /// 写停滞超时：一个写块在超时窗口内没有进展。
    #[error("写串口超时")]
    WriteTimeout,
    /// 设备不存在。
    #[error("串口不存在")]
    NotFound,
    /// 无权限打开串口。
    #[error("无权限打开串口")]
    PermissionDenied,
    /// 串口被其他进程占用。
    #[error("串口被占用")]
    Busy,
    /// 未归类的 I/O 错误。
    #[error("串口 I/O 错误：{0}")]
    Io(String),
}
