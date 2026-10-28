//! 开机自启：Windows 注册表 Run 键 / Linux 用户 autostart `.desktop`。
//!
//! - 启用时记录当前可执行文件绝对路径；每次启动若已启用则刷新路径（免安装包移动后仍有效）。
//! - 自启仅“登录后启动进程”，不打开串口、不恢复会话。

use std::io;
use std::path::Path;

/// 应用名（注册表值名 / `.desktop` 文件名）。
pub const APP_NAME: &str = "WebSocket2Serial";
/// Windows 当前用户 Run 键路径。
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Windows Run 键值：带 `--minimized` 的可执行文件命令行。
///
/// 含空格/制表符的路径用双引号包裹，保证被系统当作单个命令行参数解析。
pub fn windows_run_value(path: &Path) -> String {
    let path = path.display().to_string();
    if path.contains(' ') || path.contains('\t') {
        format!("\"{path}\" --minimized")
    } else {
        format!("{path} --minimized")
    }
}

/// Linux `.desktop` 自启条目内容（纯函数，可跨平台测试）。
pub fn desktop_entry(exe_path: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Comment=WebSocket↔串口 本机常驻透传工具\n\
         Exec={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        APP_NAME,
        desktop_exec_quote(&exe_path.display().to_string())
    )
}

/// Desktop Entry 规范 `Exec=` 参数的引号/转义。
///
/// 参数含保留字符（空格、制表符、引号、反斜杠、反引号、`$` 等）时整体加双引号，
/// 并在引号内对 `"`、`` ` ``、`$`、`\` 前置反斜杠转义。
fn desktop_exec_quote(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    if arg.is_empty() || arg.chars().any(|c| RESERVED.contains(&c)) {
        let mut out = String::with_capacity(arg.len() + 2);
        out.push('"');
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        arg.to_string()
    }
}

/// 当前自启状态。
pub fn autostart_enabled() -> io::Result<bool> {
    #[cfg(windows)]
    {
        windows_enabled()
    }
    #[cfg(not(windows))]
    {
        Ok(desktop_path().exists())
    }
}

/// 设置自启开关；`true` 时写当前可执行文件绝对路径。
pub fn set_autostart(enabled: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows_set(enabled)
    }
    #[cfg(not(windows))]
    {
        if enabled {
            let path = desktop_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, desktop_entry(&std::env::current_exe()?))
        } else {
            match std::fs::remove_file(desktop_path()) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            }
        }
    }
}

/// 启动时若已启用，刷新当前 exe 路径；返回刷新后是否启用。
pub fn refresh_if_enabled() -> io::Result<bool> {
    if autostart_enabled()? {
        set_autostart(true)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Linux 自启 `.desktop` 路径。
#[cfg(not(windows))]
fn desktop_path() -> std::path::PathBuf {
    dirs::config_dir()
        .map(|dir| dir.join("autostart").join("websocket2serial.desktop"))
        .unwrap_or_else(|| std::path::PathBuf::from(".config/autostart/websocket2serial.desktop"))
}

/// Windows 注册表 Run 键：读取是否启用。
#[cfg(windows)]
fn windows_enabled() -> io::Result<bool> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = match hkcu.open_subkey(RUN_KEY) {
        Ok(run) => run,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    match run.get_value::<String, _>(APP_NAME) {
        Ok(value) => Ok(!value.is_empty()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Windows 注册表 Run 键：写入/删除。
#[cfg(windows)]
fn windows_set(enabled: bool) -> io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu.create_subkey(RUN_KEY)?;
    if enabled {
        let exe = std::env::current_exe()?;
        run.set_value(APP_NAME, &windows_run_value(&exe))
    } else {
        match run.delete_value(APP_NAME) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}
