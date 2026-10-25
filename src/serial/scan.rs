//! 串口扫描与端口差集。

use std::io;

use serialport::{ErrorKind, SerialPortInfo, SerialPortType};

use crate::protocol::PortInfo;

use super::transport::TransportError;

/// 系统串口扫描器。
pub trait PortScanner: Send + Sync {
    fn scan(&self) -> Result<Vec<PortInfo>, TransportError>;
}

/// 基于 `serialport::available_ports()` 的扫描器。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemScanner;

impl SystemScanner {
    pub fn new() -> Self {
        Self
    }
}

impl PortScanner for SystemScanner {
    fn scan(&self) -> Result<Vec<PortInfo>, TransportError> {
        serialport::available_ports()
            .map(|ports| ports.into_iter().map(map_port).collect())
            .map_err(map_scan_error)
    }
}

/// 两次扫描结果的差集，按 `path` 判定增删。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortDiff {
    pub added: Vec<PortInfo>,
    pub removed: Vec<String>,
}

impl PortDiff {
    /// 计算 `previous → current` 的新增与移除端口。
    pub fn between(previous: &[PortInfo], current: &[PortInfo]) -> Self {
        let added = current
            .iter()
            .filter(|new| !previous.iter().any(|old| old.path == new.path))
            .cloned()
            .collect();
        let removed = previous
            .iter()
            .filter(|old| !current.iter().any(|new| new.path == old.path))
            .map(|old| old.path.clone())
            .collect();
        Self { added, removed }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

fn map_port(info: SerialPortInfo) -> PortInfo {
    match info.port_type {
        SerialPortType::UsbPort(usb) => PortInfo {
            path: info.port_name,
            port_type: Some("usb".to_string()),
            vid: Some(usb.vid),
            pid: Some(usb.pid),
            serial_number: usb.serial_number,
            manufacturer: usb.manufacturer,
            product: usb.product,
        },
        SerialPortType::PciPort => basic_port(info.port_name, "pci"),
        SerialPortType::BluetoothPort => basic_port(info.port_name, "bluetooth"),
        SerialPortType::Unknown => basic_port(info.port_name, "unknown"),
    }
}

fn basic_port(path: String, port_type: &str) -> PortInfo {
    PortInfo {
        path,
        port_type: Some(port_type.to_string()),
        vid: None,
        pid: None,
        serial_number: None,
        manufacturer: None,
        product: None,
    }
}

fn map_scan_error(error: serialport::Error) -> TransportError {
    match error.kind() {
        ErrorKind::NoDevice => TransportError::NotFound,
        ErrorKind::Io(kind) => match kind {
            io::ErrorKind::NotFound => TransportError::NotFound,
            io::ErrorKind::PermissionDenied => TransportError::PermissionDenied,
            _ => TransportError::Io(error.to_string()),
        },
        ErrorKind::InvalidInput | ErrorKind::Unknown => TransportError::Io(error.to_string()),
    }
}
