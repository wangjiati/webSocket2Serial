//! 串口参数校验与端口差集。

mod common;

use websocket2serial::protocol::{FlowControl, Parity, PortInfo};
use websocket2serial::serial::{PortDiff, SerialParams};

fn sample_params(path: &str) -> SerialParams {
    SerialParams {
        path: path.to_string(),
        baud_rate: 115200,
        data_bits: 8,
        parity: Parity::None,
        stop_bits: 1,
        flow_control: FlowControl::None,
        dtr: None,
        rts: None,
    }
}

fn sample_port(path: &str) -> PortInfo {
    PortInfo {
        path: path.to_string(),
        port_type: Some("usb".to_string()),
        vid: Some(0x0403),
        pid: Some(0x6001),
        serial_number: Some("A123".to_string()),
        manufacturer: Some("FTDI".to_string()),
        product: Some("USB Serial".to_string()),
    }
}

#[test]
fn rejects_invalid_serial_params() {
    let mut p = sample_params("COM3");
    p.baud_rate = 0;
    assert!(p.validate().is_err());
    p.baud_rate = 115200;
    p.data_bits = 4;
    assert!(p.validate().is_err());
    p.data_bits = 8;
    p.stop_bits = 3;
    assert!(p.validate().is_err());
    p.stop_bits = 1;
    p.path = "".into();
    assert!(p.validate().is_err());
}

#[test]
fn port_diff_detects_added_and_removed() {
    let a = sample_port("COM3");
    let b = sample_port("COM4");
    let d = PortDiff::between(&[a.clone()], &[b.clone()]);
    assert_eq!(d.added, vec![b]);
    assert_eq!(d.removed, vec![a.path]);
}

#[test]
fn rejects_mark_space_parity() {
    for parity in [Parity::Mark, Parity::Space] {
        let mut p = sample_params("COM3");
        p.parity = parity;
        assert!(p.validate().is_err());
    }
    for parity in [Parity::None, Parity::Odd, Parity::Even] {
        let mut p = sample_params("COM3");
        p.parity = parity;
        assert!(p.validate().is_ok());
    }
}
