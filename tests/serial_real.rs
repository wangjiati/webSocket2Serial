//! 真实串口传输：`io::ErrorKind` 错误映射 +（ignored 的）真实串口回环用例。
//!
//! 真实串口用例需要 `WS2S_TEST_PORT` 指向一只已短接 TX/RX 的串口或虚拟串口，
//! 且默认被 `#[ignore]`，避免在没有硬件的环境下误报失败。

use std::io;
use std::time::Duration;

use websocket2serial::protocol::{FlowControl, Parity};
use websocket2serial::serial::{
    map_io_error, SerialParams, SystemTransportFactory, TransportError, TransportFactory,
};

/// 构造一组可用的串口参数；真实用例据此打开串口。
fn sample_params(path: &str) -> SerialParams {
    SerialParams {
        path: path.to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        parity: Parity::None,
        stop_bits: 1,
        flow_control: FlowControl::None,
        dtr: Some(true),
        rts: Some(false),
    }
}

#[test]
fn maps_io_errors_to_transport_errors() {
    assert!(matches!(
        map_io_error(io::ErrorKind::TimedOut),
        TransportError::Timeout
    ));
    assert!(matches!(
        map_io_error(io::ErrorKind::BrokenPipe),
        TransportError::DeviceLost
    ));
    assert!(matches!(
        map_io_error(io::ErrorKind::NotFound),
        TransportError::NotFound
    ));
    assert!(matches!(
        map_io_error(io::ErrorKind::PermissionDenied),
        TransportError::PermissionDenied
    ));
    assert!(matches!(
        map_io_error(io::ErrorKind::Other),
        TransportError::Io(_)
    ));
}

#[tokio::test]
#[ignore = "需要 WS2S_TEST_PORT 指向一只真实/虚拟串口"]
async fn real_serial_echo() {
    let path = std::env::var("WS2S_TEST_PORT").unwrap();
    let mut t = SystemTransportFactory::new(Duration::from_millis(5), Duration::from_secs(5))
        .open(&sample_params(&path))
        .unwrap();
    t.write(b"ping", Duration::from_secs(1)).unwrap();
    let mut buf = [0u8; 4];
    let n = t.read(&mut buf, Duration::from_secs(1)).unwrap();
    assert_eq!(&buf[..n], b"ping");
}
