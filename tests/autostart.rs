//! 自启 .desktop 条目内容测试（纯函数，跨平台）。

use std::path::Path;

use websocket2serial::platform::autostart::desktop_entry;

#[test]
fn desktop_entry_contains_absolute_exec_and_autostart() {
    let e = desktop_entry(Path::new("/home/u/.local/bin/websocket2serial"));
    assert!(e.contains("Exec=/home/u/.local/bin/websocket2serial"));
    assert!(e.contains("X-GNOME-Autostart-enabled=true"));
}
