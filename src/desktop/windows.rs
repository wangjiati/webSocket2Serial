//! Win32 窗口与 WebView2 宿主。所有 COM/窗口对象均留在专用 UI 线程。

use std::path::Path;
use std::ptr::null_mut;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use webview2_com::{
    CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, Microsoft::Web::WebView2::Win32::*,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{E_POINTER, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::EventRegistrationToken;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::state::{navigation_action, NavigationAction};
use super::DesktopCommand;
use crate::paths;

const WINDOW_CLASS: &str = "WebSocket2SerialDesktop";
const WM_SHOW_DESKTOP: u32 = WM_APP + 20;

struct WebViewHost {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    navigation_token: EventRegistrationToken,
    new_window_token: EventRegistrationToken,
}

impl Drop for WebViewHost {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .webview
                .remove_NavigationStarting(self.navigation_token);
            let _ = self
                .webview
                .remove_NewWindowRequested(self.new_window_token);
            let _ = self.controller.Close();
        }
    }
}

struct DesktopContext {
    hwnd: HWND,
    command_tx: mpsc::Sender<DesktopCommand>,
    admin_url: Arc<Mutex<String>>,
    webview: Option<WebViewHost>,
}

pub(super) fn run(
    admin_url: String,
    visible: bool,
    command_tx: mpsc::Sender<DesktopCommand>,
    command_rx: mpsc::Receiver<DesktopCommand>,
) {
    unsafe {
        if let Err(err) = CoInitializeEx(None, COINIT_APARTMENTTHREADED) {
            tracing::error!(error = ?err, "桌面 UI COM 初始化失败");
            return;
        }
    }

    let class_name = wide(WINDOW_CLASS);
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: unsafe { GetModuleHandleW(None).unwrap_or_default() },
        lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        tracing::warn!(error = ?windows::core::Error::from_win32(), "桌面窗口类注册失败");
        unsafe { CoUninitialize() };
        return;
    }

    let admin_url = Arc::new(Mutex::new(admin_url));
    let mut ctx = Box::new(DesktopContext {
        hwnd: HWND::default(),
        command_tx,
        admin_url,
        webview: None,
    });
    let ctx_ptr = (&mut *ctx) as *mut DesktopContext;
    let title = wide("WebSocket2Serial");
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            720,
            None,
            None,
            class.hInstance,
            Some(ctx_ptr.cast()),
        )
    };
    if hwnd.0 == 0 {
        tracing::warn!(error = ?windows::core::Error::from_win32(), "桌面窗口创建失败");
        unsafe { CoUninitialize() };
        return;
    }
    ctx.hwnd = hwnd;

    if visible {
        show_desktop(&mut ctx);
    }

    let mut running = true;
    while running {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                DesktopCommand::Show(url) => {
                    *ctx.admin_url.lock().unwrap() = url;
                    show_desktop(&mut ctx);
                }
                DesktopCommand::Hide => unsafe {
                    ShowWindow(ctx.hwnd, SW_HIDE);
                },
                DesktopCommand::Shutdown => {
                    running = false;
                    unsafe {
                        ctx.webview.take();
                        DestroyWindow(ctx.hwnd);
                    }
                    break;
                }
            }
        }
        if !running {
            break;
        }

        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.into() {
            if msg.message == WM_QUIT {
                running = false;
                break;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    drop(ctx);
    unsafe { CoUninitialize() };
}

fn show_desktop(ctx: &mut DesktopContext) {
    if ctx.webview.is_none() {
        match create_webview(ctx.hwnd, Arc::clone(&ctx.admin_url)) {
            Ok(host) => ctx.webview = Some(host),
            Err(err) => {
                tracing::warn!(error = %err, "内嵌管理窗口初始化失败");
                offer_browser_fallback(ctx);
                return;
            }
        }
    }

    let url = ctx.admin_url.lock().unwrap().clone();
    if let Some(host) = &ctx.webview {
        let url_wide = wide(&url);
        if let Err(err) = unsafe { host.webview.Navigate(PCWSTR::from_raw(url_wide.as_ptr())) } {
            tracing::warn!(error = ?err, "管理页导航失败");
            offer_browser_fallback(ctx);
            return;
        }
        resize_webview(ctx.hwnd, &host.controller);
    }
    unsafe {
        ShowWindow(ctx.hwnd, SW_RESTORE);
        ShowWindow(ctx.hwnd, SW_SHOW);
        SetForegroundWindow(ctx.hwnd);
    }
}

fn offer_browser_fallback(ctx: &DesktopContext) {
    let text = wide(
        "无法启动内嵌管理窗口。请确认 WebView2 Runtime 可用。\n\n是否改用系统浏览器打开管理页？",
    );
    let title = wide("WebSocket2Serial");
    let answer = unsafe {
        MessageBoxW(
            ctx.hwnd,
            PCWSTR::from_raw(text.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            MB_YESNO | MB_ICONERROR | MB_SETFOREGROUND,
        )
    };
    if answer == IDYES {
        let url = ctx.admin_url.lock().unwrap().clone();
        if let Err(err) = open::that(&url) {
            tracing::warn!(error = %err, "系统浏览器打开管理页失败");
        }
    }
}

fn create_webview(hwnd: HWND, admin_url: Arc<Mutex<String>>) -> Result<WebViewHost, String> {
    let environment = match create_environment(None) {
        Ok(environment) => environment,
        Err(system_err) => {
            let fixed = paths::bundled_webview2_dir();
            if !fixed.join("msedgewebview2.exe").is_file() {
                return Err(system_err);
            }
            create_environment(Some(&fixed)).map_err(|fixed_err| {
                format!("系统 Runtime: {system_err}; 固定 Runtime: {fixed_err}")
            })?
        }
    };

    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            environment
                .CreateCoreWebView2Controller(hwnd, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error_code, controller| {
            error_code?;
            tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                .map_err(|_| windows::core::Error::from(E_POINTER))?;
            Ok(())
        }),
    )
    .map_err(|err| format!("创建 WebView2 控制器失败：{err:?}"))?;
    let controller = rx
        .recv()
        .map_err(|err| format!("等待 WebView2 控制器失败：{err}"))?
        .map_err(|err| format!("创建 WebView2 控制器失败：{err:?}"))?;
    let webview = unsafe { controller.CoreWebView2() }
        .map_err(|err| format!("获取 WebView2 失败：{err:?}"))?;
    unsafe {
        controller
            .SetIsVisible(true)
            .map_err(|err| err.to_string())?;
        let settings = webview.Settings().map_err(|err| err.to_string())?;
        settings
            .SetAreDevToolsEnabled(false)
            .map_err(|err| err.to_string())?;
    }

    let navigation_url = Arc::clone(&admin_url);
    let navigation_handler =
        NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                let mut raw = PWSTR(null_mut());
                if unsafe { args.Uri(&mut raw) }.is_ok() {
                    let target = CoTaskMemPWSTR::from(raw).to_string();
                    let base = navigation_url.lock().unwrap().clone();
                    match navigation_action(&base, &target) {
                        NavigationAction::Allow => {}
                        NavigationAction::OpenExternal => {
                            unsafe { args.SetCancel(true) }?;
                            open_external(&target);
                        }
                        NavigationAction::Block => unsafe { args.SetCancel(true) }?,
                    }
                }
            }
            Ok(())
        }));
    let new_window_handler = NewWindowRequestedEventHandler::create(Box::new(
        move |_sender, args| {
            if let Some(args) = args {
                unsafe { args.SetHandled(true) }?;
                let mut raw = PWSTR(null_mut());
                if unsafe { args.Uri(&mut raw) }.is_ok() {
                    let target = CoTaskMemPWSTR::from(raw).to_string();
                    if matches!(url::Url::parse(&target), Ok(url) if matches!(url.scheme(), "http" | "https"))
                    {
                        open_external(&target);
                    }
                }
            }
            Ok(())
        },
    ));
    let mut navigation_token = EventRegistrationToken::default();
    let mut new_window_token = EventRegistrationToken::default();
    unsafe {
        webview
            .add_NavigationStarting(&navigation_handler, &mut navigation_token)
            .map_err(|err| err.to_string())?;
        webview
            .add_NewWindowRequested(&new_window_handler, &mut new_window_token)
            .map_err(|err| err.to_string())?;
    }

    let host = WebViewHost {
        controller,
        webview,
        navigation_token,
        new_window_token,
    };
    resize_webview(hwnd, &host.controller);
    Ok(host)
}

fn create_environment(browser: Option<&Path>) -> Result<ICoreWebView2Environment, String> {
    let data_dir = paths::webview_data_dir();
    std::fs::create_dir_all(&data_dir)
        .map_err(|err| format!("创建 WebView2 数据目录失败：{err}"))?;
    let data = wide(&data_dir.to_string_lossy());
    let browser = browser.map(|path| wide(&path.to_string_lossy()));
    let browser_ptr = browser
        .as_ref()
        .map_or(PCWSTR::null(), |path| PCWSTR::from_raw(path.as_ptr()));
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                browser_ptr,
                PCWSTR::from_raw(data.as_ptr()),
                None,
                &handler,
            )
            .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error_code, environment| {
            error_code?;
            tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                .map_err(|_| windows::core::Error::from(E_POINTER))?;
            Ok(())
        }),
    )
    .map_err(|err| format!("创建 WebView2 环境失败：{err:?}"))?;
    rx.recv()
        .map_err(|err| format!("等待 WebView2 环境失败：{err}"))?
        .map_err(|err| format!("创建 WebView2 环境失败：{err:?}"))
}

fn resize_webview(hwnd: HWND, controller: &ICoreWebView2Controller) {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
        let _ = controller.SetBounds(rect);
    }
}

fn open_external(url: &str) {
    if let Err(err) = open::that(url) {
        tracing::warn!(error = %err, url, "打开外部地址失败");
    }
}

pub(super) fn wake_existing() -> bool {
    let class = wide(WINDOW_CLASS);
    unsafe {
        let hwnd = FindWindowW(PCWSTR::from_raw(class.as_ptr()), PCWSTR::null());
        if hwnd.0 == 0 {
            return false;
        }
        PostMessageW(hwnd, WM_SHOW_DESKTOP, WPARAM(0), LPARAM(0)).as_bool()
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DesktopContext;
    if ctx.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ctx = &mut *ctx;
    match msg {
        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_SIZE if wparam.0 as u32 == SIZE_MINIMIZED => {
            ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(host) = &ctx.webview {
                resize_webview(hwnd, &host.controller);
            }
            LRESULT(0)
        }
        WM_SHOW_DESKTOP => {
            let url = ctx.admin_url.lock().unwrap().clone();
            let _ = ctx.command_tx.send(DesktopCommand::Show(url));
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
