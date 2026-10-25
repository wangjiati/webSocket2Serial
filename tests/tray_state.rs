//! 托盘状态 → 颜色 / 提示文本的纯函数测试。

use websocket2serial::tray::state::tooltip_text;
use websocket2serial::tray::TrayStatus;

#[test]
fn tray_status_maps_to_expected_color() {
    assert_eq!(TrayStatus::Idle.color(), 0x00808080);
    assert_eq!(TrayStatus::Active.color(), 0x0000A000);
    assert_eq!(TrayStatus::Waiting.color(), 0x00D0A000);
    assert_eq!(TrayStatus::Error.color(), 0x00C00000);
}

#[test]
fn tooltip_text_includes_version_port_sessions_and_last_error() {
    let t = tooltip_text("0.1.0", 8765, 2, Some("DEVICE_LOST"));
    assert!(t.contains("0.1.0"));
    assert!(t.contains("8765"));
    assert!(t.contains("2 个会话"));
    assert!(t.contains("DEVICE_LOST"));
}
