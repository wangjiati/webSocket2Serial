//! 会话状态、计数不变式与调试环形缓冲。

use tokio::sync::{broadcast, mpsc};

use websocket2serial::config::SessionConfig;
use websocket2serial::protocol::{Counters, FlowControl, Parity};
use websocket2serial::serial::SerialParams;
use websocket2serial::session::{DebugFrame, DebugRing, Session};

/// 构造一组可用的串口参数。
fn sample_params(path: &str) -> SerialParams {
    SerialParams {
        path: path.to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        parity: Parity::None,
        stop_bits: 1,
        flow_control: FlowControl::None,
        dtr: None,
        rts: None,
    }
}

#[test]
fn token_rotation_invalidates_old_token() {
    let (events, _) = broadcast::channel(16);
    let (closed_tx, _closed_rx) = mpsc::unbounded_channel();
    let s = Session::new(
        "s1",
        sample_params("COM3"),
        SessionConfig::default(),
        events,
        closed_tx,
    );
    let old = s.current_token();
    let new = s.rotate_token();
    assert_ne!(old, new);
    assert!(!s.check_token(&old));
    assert!(s.check_token(&new));
}

#[test]
fn counters_keep_invariant() {
    let mut c = Counters::default();
    c.add_rx(100);
    c.add_discarded(30);
    c.add_tx(10);
    assert_eq!(c.delivered(), 70);
    assert_eq!(c.rx_bytes, 100);
    assert_eq!(c.rx_discarded_bytes, 30);
    assert!(c.rx_bytes >= c.rx_discarded_bytes);
}

#[test]
fn debug_ring_is_bounded_and_seq_increases() {
    let mut r = DebugRing::new(64);
    for i in 0..10 {
        r.push(DebugFrame::new("rx", i, i, vec![0u8; 16]));
    }
    let frames = r.snapshot();
    assert!(frames.len() < 10);
    assert!(frames.windows(2).all(|w| w[0].seq + 1 == w[1].seq));
}
