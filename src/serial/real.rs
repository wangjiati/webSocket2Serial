//! 真实串口传输：通过 `serialport` 打开设备并实现 [`SerialTransport`]。
//!
//! 打开在调用线程阻塞执行（上层用 `spawn_blocking` 包裹）；`read`/`write` 每次调用前
//! 先把底层驱动超时设为传入的 `timeout`，从而在同一个端口上分别支持读/写两种超时。

use std::io;
use std::time::Duration;

use serialport::{
    DataBits, ErrorKind, FlowControl as SerialFlowControl, Parity as SerialParity, SerialPort,
    StopBits,
};

use super::params::SerialParams;
use super::transport::{SerialTransport, TransportError, TransportFactory};
use crate::protocol::{FlowControl, Parity};

/// 基于系统真实串口的传输工厂。
///
/// `read_timeout` 作为打开后的初始超时；`write_timeout` 随传输对象保留，供上层/会话
/// 读写线程按需引用。真正的单次读写超时仍由 [`SerialTransport`] 的 `timeout` 参数传入。
pub struct SystemTransportFactory {
    read_timeout: Duration,
    write_timeout: Duration,
}

impl SystemTransportFactory {
    /// 以读/写超时构造工厂。
    pub fn new(read_timeout: Duration, write_timeout: Duration) -> Self {
        Self {
            read_timeout,
            write_timeout,
        }
    }
}

impl TransportFactory for SystemTransportFactory {
    fn open(&self, params: &SerialParams) -> Result<Box<dyn SerialTransport>, TransportError> {
        params
            .validate()
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let mut builder = serialport::new(&params.path, params.baud_rate)
            .data_bits(map_data_bits(params.data_bits))
            .parity(map_parity(params.parity)?)
            .stop_bits(map_stop_bits(params.stop_bits))
            .flow_control(map_flow_control(params.flow_control))
            .timeout(self.read_timeout);
        if let Some(dtr) = params.dtr {
            builder = builder.dtr_on_open(dtr);
        }

        let port = builder
            .open()
            .map_err(|e| map_open_error(e, &params.path))?;
        let mut transport = RealTransport {
            port,
            read_timeout: self.read_timeout,
            write_timeout: self.write_timeout,
        };
        // 初始 RTS：builder 只支持 dtr_on_open，RTS 需在打开后单独设置。
        if let Some(rts) = params.rts {
            transport.set_rts(rts)?;
        }

        Ok(Box::new(transport))
    }
}

/// 包装 `Box<dyn serialport::SerialPort>` 的传输实现。
pub struct RealTransport {
    port: Box<dyn SerialPort>,
    read_timeout: Duration,
    write_timeout: Duration,
}

impl RealTransport {
    /// 工厂配置的读超时。
    pub fn read_timeout(&self) -> Duration {
        self.read_timeout
    }

    /// 工厂配置的写超时。
    pub fn write_timeout(&self) -> Duration {
        self.write_timeout
    }
}

impl SerialTransport for RealTransport {
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        self.port.set_timeout(timeout).map_err(map_serial_error)?;
        self.port
            .read(buf)
            .map_err(|e| map_io_error_full(e, TransportError::Timeout))
    }

    fn write(&mut self, buf: &[u8], timeout: Duration) -> Result<usize, TransportError> {
        self.port.set_timeout(timeout).map_err(map_serial_error)?;
        self.port
            .write(buf)
            .map_err(|e| map_io_error_full(e, TransportError::WriteTimeout))
    }

    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError> {
        self.port
            .write_data_terminal_ready(level)
            .map_err(map_serial_error)
    }

    fn set_rts(&mut self, level: bool) -> Result<(), TransportError> {
        self.port
            .write_request_to_send(level)
            .map_err(map_serial_error)
    }
}

/// 把 [`io::ErrorKind`] 映射为 [`TransportError`]。
///
/// `TimedOut` 默认按读超时处理；写路径在 [`map_io_error_full`] 里覆盖为 `WriteTimeout`。
pub fn map_io_error(kind: io::ErrorKind) -> TransportError {
    match kind {
        io::ErrorKind::TimedOut => TransportError::Timeout,
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset => TransportError::DeviceLost,
        io::ErrorKind::NotFound => TransportError::NotFound,
        io::ErrorKind::PermissionDenied => TransportError::PermissionDenied,
        other => TransportError::Io(format!("{other:?}")),
    }
}

/// 把完整的 `io::Error` 映射为 [`TransportError`]，携带底层错误描述并按读写区分超时。
fn map_io_error_full(error: io::Error, on_timeout: TransportError) -> TransportError {
    // 尽力识别“被占用”：Windows 共享冲突 / Linux EBUSY。
    if is_busy(&error) {
        return TransportError::Busy;
    }
    if error.kind() == io::ErrorKind::TimedOut {
        return on_timeout;
    }
    match map_io_error(error.kind()) {
        // 未归类的错误保留底层描述，比只留 kind 更有诊断价值。
        TransportError::Io(_) => TransportError::Io(error.to_string()),
        other => other,
    }
}

/// 打开串口失败的映射：serialport 会把“被占用”与 Windows 下的“不存在”都归一为 `NoDevice`。
fn map_open_error(error: serialport::Error, path: &str) -> TransportError {
    match error.kind() {
        // Windows 下 `ERROR_FILE_NOT_FOUND`/`ERROR_ACCESS_DENIED` 都落为 NoDevice，
        // 用端口枚举结果消歧“不存在 vs 被占用”。
        ErrorKind::NoDevice => {
            let available = serialport::available_ports()
                .ok()
                .map(|ports| ports.into_iter().map(|p| p.port_name).collect::<Vec<_>>());
            classify_no_device(path, available.as_deref())
        }
        ErrorKind::Io(io::ErrorKind::NotFound) => TransportError::NotFound,
        ErrorKind::Io(io::ErrorKind::PermissionDenied) => TransportError::PermissionDenied,
        _ => TransportError::Io(error.to_string()),
    }
}

/// 纯函数：用端口枚举结果消歧 `NoDevice`。
///
/// - `available = None`：枚举失败，无法判断，归 `Io`。
/// - `path` 在列表中：端口存在但打不开 → `Busy`。
/// - `path` 不在列表中：端口不存在 → `NotFound`。
fn classify_no_device(path: &str, available: Option<&[String]>) -> TransportError {
    match available {
        None => TransportError::Io("打开失败且无法枚举端口以区分不存在/占用".to_string()),
        Some(paths) if paths.iter().any(|p| p == path) => TransportError::Busy,
        Some(_) => TransportError::NotFound,
    }
}

/// `set_timeout`/`set_dtr`/`set_rts` 等端口操作失败的映射。
fn map_serial_error(error: serialport::Error) -> TransportError {
    match error.kind() {
        ErrorKind::NoDevice => TransportError::DeviceLost,
        ErrorKind::Io(io::ErrorKind::NotFound) => TransportError::NotFound,
        ErrorKind::Io(io::ErrorKind::PermissionDenied) => TransportError::PermissionDenied,
        ErrorKind::Io(io::ErrorKind::TimedOut) => TransportError::Timeout,
        _ => TransportError::Io(error.to_string()),
    }
}

/// 尽力判断 `io::Error` 是否为“串口被其他进程占用”。
fn is_busy(error: &io::Error) -> bool {
    let Some(code) = error.raw_os_error() else {
        return false;
    };
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33)
        code == 32 || code == 33
    }
    #[cfg(unix)]
    {
        // EBUSY：Linux 及多数 Unix 平台均为 16
        code == 16
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = code;
        false
    }
}

fn map_data_bits(bits: u8) -> DataBits {
    DataBits::try_from(bits).expect("数据位已由 SerialParams::validate 校验为 5..=8")
}

fn map_stop_bits(bits: u8) -> StopBits {
    StopBits::try_from(bits).expect("停止位已由 SerialParams::validate 校验为 1 或 2")
}

fn map_parity(parity: Parity) -> Result<SerialParity, TransportError> {
    match parity {
        Parity::None => Ok(SerialParity::None),
        Parity::Odd => Ok(SerialParity::Odd),
        Parity::Even => Ok(SerialParity::Even),
        // serialport 4.10.1 只支持 none/odd/even；mark/space 无底层对应值。
        Parity::Mark | Parity::Space => Err(TransportError::Io(
            "serialport 不支持 mark/space 校验位".to_string(),
        )),
    }
}

fn map_flow_control(flow_control: FlowControl) -> SerialFlowControl {
    match flow_control {
        FlowControl::None => SerialFlowControl::None,
        FlowControl::Software => SerialFlowControl::Software,
        FlowControl::Hardware => SerialFlowControl::Hardware,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_err(kind: io::ErrorKind) -> io::Error {
        io::Error::new(kind, "test")
    }

    #[test]
    fn map_io_error_full_distinguishes_read_write_timeout() {
        // 读路径：调用方传 Timeout。
        assert_eq!(
            map_io_error_full(io_err(io::ErrorKind::TimedOut), TransportError::Timeout),
            TransportError::Timeout
        );
        // 写路径：调用方传 WriteTimeout。
        assert_eq!(
            map_io_error_full(
                io_err(io::ErrorKind::TimedOut),
                TransportError::WriteTimeout
            ),
            TransportError::WriteTimeout
        );
    }

    #[test]
    fn classify_no_device_disambiguates() {
        let available = ["COM1", "COM3"].map(String::from);

        // 不在列表 → NotFound
        assert_eq!(
            classify_no_device("COM9", Some(available.as_slice())),
            TransportError::NotFound
        );
        // 在列表 → Busy
        assert_eq!(
            classify_no_device("COM3", Some(available.as_slice())),
            TransportError::Busy
        );
        // 枚举失败 → Io
        assert!(matches!(
            classify_no_device("COM3", None),
            TransportError::Io(_)
        ));
    }

    #[test]
    fn map_open_error_maps_not_found_and_permission() {
        let not_found = serialport::Error::new(
            serialport::ErrorKind::Io(io::ErrorKind::NotFound),
            "missing",
        );
        assert_eq!(map_open_error(not_found, "COM3"), TransportError::NotFound);

        let denied = serialport::Error::new(
            serialport::ErrorKind::Io(io::ErrorKind::PermissionDenied),
            "denied",
        );
        assert_eq!(
            map_open_error(denied, "COM3"),
            TransportError::PermissionDenied
        );
    }

    #[test]
    fn map_parity_rejects_mark_space() {
        assert_eq!(map_parity(Parity::None).unwrap(), SerialParity::None);
        assert_eq!(map_parity(Parity::Odd).unwrap(), SerialParity::Odd);
        assert_eq!(map_parity(Parity::Even).unwrap(), SerialParity::Even);
        assert!(matches!(
            map_parity(Parity::Mark),
            Err(TransportError::Io(_))
        ));
        assert!(matches!(
            map_parity(Parity::Space),
            Err(TransportError::Io(_))
        ));
    }

    #[test]
    fn map_flow_control_maps_all() {
        assert_eq!(map_flow_control(FlowControl::None), SerialFlowControl::None);
        assert_eq!(
            map_flow_control(FlowControl::Software),
            SerialFlowControl::Software
        );
        assert_eq!(
            map_flow_control(FlowControl::Hardware),
            SerialFlowControl::Hardware
        );
    }
}
