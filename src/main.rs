#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use websocket2serial::app::{build_app, ensure_admin_url};
use websocket2serial::cli::Cli;
use websocket2serial::config::{Config, OriginAllowlist};
use websocket2serial::desktop::{self, DesktopHandle};
use websocket2serial::http::AdminService;
use websocket2serial::platform::autostart;
use websocket2serial::platform::single_instance::SingleInstance;
use websocket2serial::protocol::{Event, SessionState};
use websocket2serial::serial::{PortScanner, SystemScanner, SystemTransportFactory};
use websocket2serial::session::{CloseReason, SessionManager};
use websocket2serial::tray::{self, TrayAction, TrayHandle, TrayState};
use websocket2serial::ws::AxumDataListenerFactory;
use websocket2serial::{logging, paths};

#[tokio::main]
async fn main() {
    attach_parent_console_for_cli();
    let cli = Cli::parse();

    let config_path: PathBuf = cli
        .config
        .clone()
        .unwrap_or_else(|| paths::config_dir().join("config.toml"));

    let (config, config_err) = Config::load_or_default(&config_path);

    // 初始化日志；guard 需持有到进程退出，避免日志线程提前终止。
    let _guard = match logging::init(&paths::log_dir(), &config.logging) {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("日志初始化失败：{err}");
            fatal_error(
                "WebSocket2Serial 启动失败",
                &format!("日志初始化失败：{err}"),
            );
            std::process::exit(1);
        }
    };

    if let Some(err) = config_err {
        warn!("配置解析失败，已回退到默认值：{err}");
    }

    if cli.force_admin {
        info!("--admin：本次启动强制开启管理页");
    }

    info!(
        "websocket2serial v{} 启动（配置 {}）",
        env!("CARGO_PKG_VERSION"),
        config_path.display()
    );

    // 单实例检查：已有实例则尝试打开其管理页后退出。
    let _single: Option<SingleInstance> = match SingleInstance::acquire() {
        Ok(Some(handle)) => Some(handle),
        Ok(None) => {
            if desktop::wake_existing() {
                info!("已有实例在运行，已请求恢复其管理窗口");
            } else {
                info!("已有实例在运行，尝试打开其管理页后退出");
                open_existing_admin(config.admin.port).await;
            }
            return;
        }
        Err(err) => {
            warn!(error = %err, "单实例检查失败，以无锁模式继续启动");
            None
        }
    };

    // 读取配置字段（build_app 会消费 config）。
    let read_timeout = Duration::from_millis(config.session.read_timeout_ms);
    let write_timeout = Duration::from_millis(config.session.write_timeout_ms);
    let session_cfg = config.session.clone();
    let origins = OriginAllowlist::new(&config.security.allowed_origins);
    let max_frame_bytes = config.session.tx_queue_bytes;
    let admin_port = config.admin.port;
    let admin_enabled = config.admin.enabled;
    let management_port = config.server.management_port;

    let (events_tx, _) = broadcast::channel::<Event>(256);

    // 数据监听器工厂需 manager 的弱引用，用 Arc::new_cyclic 构造 manager。
    let manager: Arc<SessionManager> = Arc::new_cyclic({
        let manager_events = events_tx.clone();
        move |weak| {
            let factory = Arc::new(AxumDataListenerFactory::new(
                weak.clone(),
                origins.clone(),
                max_frame_bytes,
            ));
            SessionManager::new(
                Arc::new(SystemTransportFactory::new(read_timeout, write_timeout)),
                factory,
                session_cfg,
                manager_events,
            )
        }
    });

    let admin = Arc::new(AdminService::new(admin_port, management_port));
    let scanner: Arc<dyn PortScanner> = Arc::new(SystemScanner::new());

    // 绑定管理 WS（127.0.0.1:8765）；失败致命。
    let app = match build_app(config, manager, admin.clone(), scanner, events_tx) {
        Ok(app) => app,
        Err(err) => {
            error!(error = %err, "管理 WS 绑定失败");
            fatal_error(
                "WebSocket2Serial 启动失败",
                &format!("无法绑定管理端口 {management_port}：{err}"),
            );
            std::process::exit(1);
        }
    };

    // 自启：刷新路径（若已启用）并同步状态到 /api/config。
    let mut autostart_enabled = match autostart::refresh_if_enabled() {
        Ok(enabled) => enabled,
        Err(err) => {
            warn!(error = %err, "自启状态检查失败");
            false
        }
    };
    admin.set_autostart_status(autostart_enabled);

    // 按配置或 --admin 启动管理页服务（失败不致命）。
    if admin_enabled || cli.force_admin {
        if let Err(err) = admin.set_enabled(true).await {
            warn!(error = %err, "管理页服务启动失败（不影响管理 WS）");
        }
    }
    // 运行时管理页开关状态：托盘菜单展示用，随托盘动作与 AdminState 事件同步。
    let mut admin_enabled = admin.status().enabled;

    // 托盘动作通道同时承载原生窗口的第二实例唤醒。
    let initial = current_tray_state(
        app.management_port,
        &HashMap::new(),
        &None,
        admin_enabled,
        autostart_enabled,
    );
    let (mut tray_handle, mut action_rx) =
        TrayHandle::start(env!("CARGO_PKG_VERSION").to_string(), initial);

    // Windows 原生管理窗口；管理服务配置为关闭时仅驻留托盘。
    let admin_url = admin.status().url;
    let mut desktop = match DesktopHandle::start(
        admin_url,
        admin_enabled && !cli.minimized,
        tray_handle.action_sender(),
    ) {
        Ok(desktop) => Some(desktop),
        Err(err) => {
            warn!(error = %err, "原生窗口线程启动失败，以无窗口模式运行");
            None
        }
    };

    // 退出信号：ctrl-c（全平台）+ SIGTERM（Unix）。
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    #[cfg(unix)]
    let sigterm_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(()).await;
    });
    #[cfg(unix)]
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("无法安装 SIGTERM 处理器");
        sigterm.recv().await;
        let _ = sigterm_tx.send(()).await;
    });

    // 会话状态跟踪（供托盘状态/气泡）。
    let mut event_rx = app.state.events.subscribe();
    let mut sessions: HashMap<String, SessionState> = HashMap::new();
    let mut last_error: Option<String> = None;

    info!(
        "websocket2serial 运行中（管理 WS {}，管理页端口 {}）",
        app.management_port, admin_port
    );

    // 事件循环：托盘动作 / 会话事件 / 退出信号。
    let mut quit = false;
    while !quit {
        tokio::select! {
            action = action_rx.recv() => {
                match action {
                    Some(TrayAction::OpenAdmin) => {
                        match ensure_admin_url(app.state.admin.as_ref()).await {
                            Ok(url) => {
                                admin_enabled = true;
                                show_admin(&desktop, &url, &tray_handle);
                            }
                            Err(err) => {
                                warn!(error = %err, "重新打开管理页失败");
                                tray_handle.bubble(err);
                            }
                        }
                        tray_handle.send_state(current_tray_state(
                            app.management_port,
                            &sessions,
                            &last_error,
                            admin_enabled,
                            autostart_enabled,
                        ));
                    }
                    Some(TrayAction::OpenBrowser) => {
                        match ensure_admin_url(app.state.admin.as_ref()).await {
                            Ok(url) => {
                                admin_enabled = true;
                                open_browser(&url, &tray_handle);
                            }
                            Err(err) => {
                                warn!(error = %err, "重新打开管理页失败");
                                tray_handle.bubble(err);
                            }
                        }
                        tray_handle.send_state(current_tray_state(
                            app.management_port,
                            &sessions,
                            &last_error,
                            admin_enabled,
                            autostart_enabled,
                        ));
                    }
                    Some(TrayAction::SetAdmin(enabled)) => {
                        let state = app.state.admin.set_enabled(enabled).await;
                        admin_enabled = state.enabled;
                        if !admin_enabled {
                            if let Some(desktop) = &desktop {
                                desktop.hide();
                            }
                        }
                        let _ = app.state.events.send(Event::AdminState(state));
                        tray_handle.send_state(current_tray_state(
                            app.management_port,
                            &sessions,
                            &last_error,
                            admin_enabled,
                            autostart_enabled,
                        ));
                    }
                    Some(TrayAction::SetAutostart(enabled)) => {
                        match autostart::set_autostart(enabled) {
                            Ok(()) => {
                                autostart_enabled = enabled;
                                admin.set_autostart_status(enabled);
                                tray_handle.send_state(current_tray_state(
                                    app.management_port,
                                    &sessions,
                                    &last_error,
                                    admin_enabled,
                                    autostart_enabled,
                                ));
                            }
                            Err(err) => {
                                warn!(error = %err, "设置自启失败");
                                tray_handle.bubble(format!("设置自启失败：{err}"));
                            }
                        }
                    }
                    Some(TrayAction::Quit) => quit = true,
                    None => quit = true,
                }
            }
            event = event_rx.recv() => {
                match event {
                    Ok(Event::SessionState(data)) => {
                        match data.state {
                            SessionState::Closed => {
                                sessions.remove(&data.session_id);
                                if let Some(reason) = &data.reason {
                                    if is_error_reason(reason) {
                                        last_error = Some(reason.clone());
                                        tray_handle.bubble(format!("会话关闭：{reason}"));
                                    }
                                }
                            }
                            SessionState::Active => {
                                sessions.insert(data.session_id.clone(), SessionState::Active);
                                // 会话恢复，清除最近错误。
                                last_error = None;
                            }
                            other => {
                                sessions.insert(data.session_id.clone(), other);
                            }
                        }
                        tray_handle.send_state(current_tray_state(
                            app.management_port,
                            &sessions,
                            &last_error,
                            admin_enabled,
                            autostart_enabled,
                        ));
                    }
                    Ok(Event::AdminState(state)) => {
                        // 管理页开关（含网页 admin.set）同步到托盘菜单。
                        admin_enabled = state.enabled;
                        if !admin_enabled {
                            if let Some(desktop) = &desktop {
                                desktop.hide();
                            }
                        }
                        tray_handle.send_state(current_tray_state(
                            app.management_port,
                            &sessions,
                            &last_error,
                            admin_enabled,
                            autostart_enabled,
                        ));
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "事件接收滞后，按管理服务真实状态重新同步");
                        let state = app.state.admin.status();
                        admin_enabled = state.enabled;
                        if !admin_enabled {
                            if let Some(desktop) = &desktop {
                                desktop.hide();
                            }
                        }
                        tray_handle.send_state(current_tray_state(
                            app.management_port,
                            &sessions,
                            &last_error,
                            admin_enabled,
                            autostart_enabled,
                        ));
                    }
                    Err(broadcast::error::RecvError::Closed) => {}
                }
            }
            _ = shutdown_rx.recv() => {
                info!("收到退出信号");
                quit = true;
            }
        }
    }

    // 退出清理：停托盘 → 停管理 WS → 关会话（1001）→ 停管理页。
    info!("正在退出：关闭原生窗口、托盘、服务与会话");
    if let Some(desktop) = &mut desktop {
        desktop.shutdown();
    }
    tray_handle.shutdown();

    let management_port = app.management_port;
    let state = Arc::clone(&app.state);
    app.shutdown();

    for session in state.manager.list().await {
        let _ = state
            .manager
            .close(&session.session_id, CloseReason::Shutdown)
            .await;
    }

    admin.shutdown().await;
    info!("websocket2serial 已退出（管理 WS {}）", management_port);
    // _guard drop 时刷新日志。
}

/// 发布版为 GUI 子系统；带参数从终端运行时附加父控制台以保留 CLI 输出。
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console_for_cli() {
    if std::env::args_os().len() <= 1 {
        return;
    }
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    let name: Vec<u16> = "CONOUT$".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
        let conout = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        if conout == INVALID_HANDLE_VALUE || conout == 0 {
            return;
        }
        // 保留 shell 重定向/管道句柄；仅在句柄为空时接入父控制台。
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        if stdout == 0 || stdout == INVALID_HANDLE_VALUE {
            SetStdHandle(STD_OUTPUT_HANDLE, conout);
        }
        let stderr = GetStdHandle(STD_ERROR_HANDLE);
        if stderr == 0 || stderr == INVALID_HANDLE_VALUE {
            SetStdHandle(STD_ERROR_HANDLE, conout);
        }
    }
}

#[cfg(any(not(windows), debug_assertions))]
fn attach_parent_console_for_cli() {}

/// Windows 优先恢复原生窗口；无窗口平台或 UI 启动失败时使用浏览器。
fn show_admin(desktop: &Option<DesktopHandle>, url: &str, tray: &TrayHandle) {
    #[cfg(windows)]
    if let Some(desktop) = desktop {
        desktop.show(url);
        return;
    }
    #[cfg(not(windows))]
    let _ = desktop;
    open_browser(url, tray);
}

fn open_browser(url: &str, tray: &TrayHandle) {
    if let Err(err) = open::that(url) {
        warn!(error = %err, "打开管理页失败");
        tray.bubble(format!("打开管理页失败：{err}"));
    }
}

/// 已有实例时尝试打开其管理页（探测端口，避免管理页被禁用时浏览器报错）。
async fn open_existing_admin(port: u16) {
    let url = format!("http://127.0.0.1:{port}");
    let listening = matches!(
        tokio::time::timeout(
            Duration::from_millis(300),
            tokio::net::TcpStream::connect(("127.0.0.1", port))
        )
        .await,
        Ok(Ok(_))
    );
    if listening {
        if let Err(err) = open::that(&url) {
            warn!(error = %err, "打开已有实例管理页失败");
            info_message("WebSocket2Serial", &format!("无法打开浏览器：{err}"));
        }
    } else {
        info_message(
            "WebSocket2Serial",
            "已有实例在运行，但管理页当前未开启。\n可通过托盘或重启时加 --admin 重新打开。",
        );
    }
}

/// 计算当前托盘状态快照。
fn current_tray_state(
    management_port: u16,
    sessions: &HashMap<String, SessionState>,
    last_error: &Option<String>,
    admin_enabled: bool,
    autostart: bool,
) -> TrayState {
    let states: Vec<SessionState> = sessions.values().copied().collect();
    let status = tray::state::status_for(&states, last_error.as_deref());
    TrayState {
        status,
        tooltip: tray::state::tooltip_text(
            env!("CARGO_PKG_VERSION"),
            management_port,
            sessions.len(),
            last_error.as_deref(),
        ),
        admin_enabled,
        autostart,
    }
}

/// 会话关闭 reason 是否为「错误」类（决定托盘红色与气泡）。
fn is_error_reason(reason: &str) -> bool {
    matches!(
        reason,
        "DEVICE_LOST" | "WRITE_TIMEOUT" | "SERIAL_IO_ERROR" | "BACKPRESSURE"
    )
}

/// 致命错误：日志 + Windows MessageBox / Linux stderr。
fn fatal_error(title: &str, text: &str) {
    message_box(title, text, true);
}

/// 提示信息：Windows MessageBox / Linux stderr。
fn info_message(title: &str, text: &str) {
    message_box(title, text, false);
}

#[cfg(windows)]
fn message_box(title: &str, text: &str, error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
    };
    let title = wide(title);
    let text = wide(text);
    let style = MB_OK
        | MB_SETFOREGROUND
        | if error {
            MB_ICONERROR
        } else {
            MB_ICONINFORMATION
        };
    unsafe {
        MessageBoxW(0, text.as_ptr(), title.as_ptr(), style);
    }
}

#[cfg(not(windows))]
fn message_box(_title: &str, text: &str, _error: bool) {
    eprintln!("{text}");
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
