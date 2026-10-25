//! 自启 `.desktop` 条目与 Windows Run 值内容测试（纯函数，跨平台）。

use std::path::Path;

use websocket2serial::platform::autostart::{desktop_entry, windows_run_value};

#[test]
fn desktop_entry_contains_absolute_exec_and_autostart() {
    let e = desktop_entry(Path::new("/home/u/.local/bin/websocket2serial"));
    assert!(e.contains("Exec=/home/u/.local/bin/websocket2serial"));
    assert!(e.contains("X-GNOME-Autostart-enabled=true"));
}

#[test]
fn desktop_entry_quotes_exec_with_spaces() {
    // 含空格的路径整体加双引号。
    let e = desktop_entry(Path::new("/opt/My Apps/websocket2serial"));
    assert!(e.contains("Exec=\"/opt/My Apps/websocket2serial\""));

    // 含引号/反斜杠/$ 时按 Desktop Entry 规范转义。
    let escaped = desktop_entry(Path::new(r#"/opt/a"b\c$d"#));
    assert!(escaped.contains(r#"Exec="/opt/a\"b\\c\$d""#));
}

#[test]
fn windows_run_value_quotes_paths_with_spaces() {
    let v = windows_run_value(Path::new(
        r"C:\Program Files\WebSocket2Serial\websocket2serial.exe",
    ));
    assert_eq!(
        v,
        r#""C:\Program Files\WebSocket2Serial\websocket2serial.exe""#
    );

    // 不含空格的路径保持原样（无需引号）。
    let plain = windows_run_value(Path::new(r"C:\bin\websocket2serial.exe"));
    assert_eq!(plain, r"C:\bin\websocket2serial.exe");
}
