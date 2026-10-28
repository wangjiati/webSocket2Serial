//! Win32 `Shell_NotifyIcon` 托盘：专用线程、隐藏窗口消息循环、菜单与 GDI 图标。
//!
//! 图标用 GDI 生成 16×16 实心方块（32bpp DIB + 单色掩码），不引入图片资源。
//! 托盘菜单动作经 [`UnboundedSender<TrayAction>`] 发给 main；状态/气泡经
//! [`Receiver<TrayUpdate>`] 从 main 收。菜单选择用 `TPM_RETURNCMD` 同步返回，
//! 避免窗口过程重入处理 `WM_COMMAND`。

use std::ffi::c_void;
use std::io;
use std::ptr::{null, null_mut};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetWindowLongPtrW, MessageBoxW,
    PeekMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW,
    TrackPopupMenu, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, HICON, ICONINFO,
    MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    PM_REMOVE, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_DESTROY,
    WM_LBUTTONDBLCLK, WM_NCCREATE, WM_QUIT, WM_RBUTTONUP, WNDCLASSW,
};

use super::TrayState;
use super::{TrayAction, TrayUpdate};

/// Shell_NotifyIcon 回调消息。
const WM_TRAY: u32 = WM_APP + 1;
/// 托盘图标 ID（仅一个）。
const TRAY_UID: u32 = 1;

/// 菜单项 ID。
const ID_STATUS: usize = 1;
const ID_OPEN: usize = 2;
const ID_ADMIN: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_ABOUT: usize = 5;
const ID_QUIT: usize = 6;
const ID_BROWSER: usize = 7;

/// 托盘线程上下文；通过窗口 GWLP_USERDATA 保存裸指针。
struct TrayContext {
    hwnd: HWND,
    action_tx: UnboundedSender<TrayAction>,
    version: String,
    state: TrayState,
    nid: NOTIFYICONDATAW,
    icon: HICON,
}

impl TrayContext {
    fn handle(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                // lParam 为图标上的鼠标事件。
                if lparam == WM_LBUTTONDBLCLK as isize {
                    let _ = self.action_tx.send(TrayAction::OpenAdmin);
                } else if lparam == WM_RBUTTONUP as isize {
                    self.show_menu();
                }
                0
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// 更新图标/tooltip/菜单状态。
    fn apply_state(&mut self, state: TrayState) {
        let new_icon = make_icon(state.status.color());
        self.state = state;
        let mut nid = self.nid;
        nid.uFlags = NIF_ICON | NIF_TIP;
        nid.hIcon = new_icon;
        copy_wide(&mut nid.szTip, &self.state.tooltip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
            if self.icon != 0 {
                DestroyIcon(self.icon);
            }
        }
        self.icon = new_icon;
    }

    /// 弹出错误气泡（NIF_INFO）。
    fn show_bubble(&mut self, message: String) {
        let mut nid = self.nid;
        nid.uFlags = NIF_INFO;
        copy_wide(&mut nid.szInfo, &message);
        copy_wide(&mut nid.szInfoTitle, "WebSocket2Serial");
        nid.dwInfoFlags = NIIF_ERROR;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    /// 右键菜单（TPM_RETURNCMD 同步返回，避免窗口过程重入）。
    fn show_menu(&mut self) {
        let menu = unsafe { CreatePopupMenu() };
        if menu == 0 {
            return;
        }

        let status = wide(&format!("状态：{}", self.state.status.label()));
        let open = wide("打开管理页");
        let browser = wide("在浏览器中打开");
        let admin = wide(if self.state.admin_enabled {
            "管理服务：关闭"
        } else {
            "管理服务：开启"
        });
        let autostart = wide(if self.state.autostart {
            "开机自启：关闭"
        } else {
            "开机自启：开启"
        });
        let about = wide("关于");
        let quit = wide("退出");

        unsafe {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, ID_STATUS, status.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, ID_OPEN, open.as_ptr());
            AppendMenuW(menu, MF_STRING, ID_BROWSER, browser.as_ptr());
            AppendMenuW(menu, MF_STRING, ID_ADMIN, admin.as_ptr());
            AppendMenuW(menu, MF_STRING, ID_AUTOSTART, autostart.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, ID_ABOUT, about.as_ptr());
            AppendMenuW(menu, MF_STRING, ID_QUIT, quit.as_ptr());
        }

        let mut point = POINT { x: 0, y: 0 };
        let cmd = unsafe {
            GetCursorPos(&mut point);
            SetForegroundWindow(self.hwnd);
            // 同步返回选中项 ID（未选中返回 0），随后统一销毁菜单。
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD,
                point.x,
                point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe { DestroyMenu(menu) };

        if cmd != 0 {
            self.on_command(cmd as usize);
        }
    }

    fn on_command(&mut self, id: usize) {
        match id {
            ID_OPEN => {
                let _ = self.action_tx.send(TrayAction::OpenAdmin);
            }
            ID_BROWSER => {
                let _ = self.action_tx.send(TrayAction::OpenBrowser);
            }
            ID_ADMIN => {
                let enabled = !self.state.admin_enabled;
                let _ = self.action_tx.send(TrayAction::SetAdmin(enabled));
            }
            ID_AUTOSTART => {
                let enabled = !self.state.autostart;
                let _ = self.action_tx.send(TrayAction::SetAutostart(enabled));
            }
            ID_ABOUT => self.show_about(),
            ID_QUIT => {
                let _ = self.action_tx.send(TrayAction::Quit);
            }
            _ => {}
        }
    }

    fn show_about(&self) {
        let text = wide(&format!(
            "WebSocket2Serial v{}\nWebSocket↔串口 本机常驻透传工具",
            self.version
        ));
        let title = wide("关于 WebSocket2Serial");
        unsafe {
            MessageBoxW(
                self.hwnd,
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
            );
        }
    }
}

/// 启动托盘线程。
pub(crate) fn spawn(
    version: String,
    initial: TrayState,
    action_tx: UnboundedSender<TrayAction>,
    update_rx: Receiver<TrayUpdate>,
) -> io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("ws2s-tray".into())
        .spawn(move || run(version, initial, action_tx, update_rx))
        .map_err(|err| io::Error::new(err.kind(), err))
}

fn run(
    version: String,
    initial: TrayState,
    action_tx: UnboundedSender<TrayAction>,
    update_rx: Receiver<TrayUpdate>,
) {
    let class_name = wide("WebSocket2SerialTray");
    if register_class(class_name.as_ptr()) == 0 {
        tracing::warn!("托盘窗口类注册失败（错误码 {})", unsafe {
            GetLastError()
        });
        return;
    }

    let mut ctx = TrayContext {
        hwnd: 0,
        action_tx,
        version,
        state: initial,
        nid: unsafe { std::mem::zeroed() },
        icon: 0,
    };
    ctx.icon = make_icon(ctx.state.status.color());

    let ctx_ptr = Box::into_raw(Box::new(ctx)) as *mut c_void;
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            GetModuleHandleW(null()),
            ctx_ptr,
        )
    };
    if hwnd == 0 {
        tracing::warn!("托盘隐藏窗口创建失败（错误码 {})", unsafe {
            GetLastError()
        });
        unsafe {
            let ctx = Box::from_raw(ctx_ptr as *mut TrayContext);
            if ctx.icon != 0 {
                DestroyIcon(ctx.icon);
            }
        }
        return;
    }

    unsafe {
        (*(ctx_ptr as *mut TrayContext)).hwnd = hwnd;
    }

    if !add_icon(ctx_ptr as *mut TrayContext) {
        unsafe {
            cleanup(ctx_ptr as *mut TrayContext);
        }
        return;
    }

    // 消息循环：非阻塞排空窗口消息 + 轮询托盘更新。
    loop {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                unsafe { cleanup(ctx_ptr as *mut TrayContext) };
                return;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let mut should_exit = false;
        while let Ok(update) = update_rx.try_recv() {
            match update {
                TrayUpdate::State(state) => {
                    unsafe { &mut *(ctx_ptr as *mut TrayContext) }.apply_state(state);
                }
                TrayUpdate::Bubble(message) => {
                    unsafe { &mut *(ctx_ptr as *mut TrayContext) }.show_bubble(message);
                }
                TrayUpdate::Shutdown => should_exit = true,
            }
        }
        if should_exit {
            unsafe { cleanup(ctx_ptr as *mut TrayContext) };
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 注册窗口类并返回成功与否（0 表示失败）。
fn register_class(class_name: *const u16) -> u16 {
    unsafe {
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(null()),
            hIcon: 0,
            hCursor: 0,
            hbrBackground: 0,
            lpszMenuName: null(),
            lpszClassName: class_name,
        };
        RegisterClassW(&wc)
    }
}

/// 添加托盘图标（NIM_ADD + NIM_SETVERSION）。
fn add_icon(ctx: *mut TrayContext) -> bool {
    unsafe {
        let ctx = &mut *ctx;
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = ctx.hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = ctx.icon;
        copy_wide(&mut nid.szTip, &ctx.state.tooltip);

        if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
            tracing::warn!("托盘图标添加失败（错误码 {})", GetLastError());
            return false;
        }
        // 版本 4 以获得标准鼠标消息（双击/右键）。
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &nid);
        ctx.nid = nid;
        true
    }
}

/// 退出清理：删除图标、销毁图标/窗口、释放上下文。
unsafe fn cleanup(ctx: *mut TrayContext) {
    let ctx = &mut *ctx;
    let mut nid = ctx.nid;
    nid.uFlags = 0;
    Shell_NotifyIconW(NIM_DELETE, &nid);
    if ctx.icon != 0 {
        DestroyIcon(ctx.icon);
    }
    if ctx.hwnd != 0 {
        DestroyWindow(ctx.hwnd);
    }
    drop(Box::from_raw(ctx));
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
    if ctx.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    (*ctx).handle(hwnd, msg, wparam, lparam)
}

/// 生成 16×16 实心方块图标（32bpp DIB + 单色掩码全 0 = 不透明）。
fn make_icon(rgb: u32) -> HICON {
    const SIZE: i32 = 16;
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = SIZE;
        bmi.bmiHeader.biHeight = -SIZE; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut c_void = null_mut();
        let hbm_color = CreateDIBSection(0, &bmi, DIB_RGB_COLORS, &mut bits, 0, 0);
        if hbm_color == 0 {
            return 0;
        }

        let r = (rgb >> 16) & 0xFF;
        let g = (rgb >> 8) & 0xFF;
        let b = rgb & 0xFF;
        // 32bpp DIB 内存为 BGRA。
        let pixel = (0xFF << 24) | (r << 16) | (g << 8) | b;
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (SIZE * SIZE) as usize);
        for p in pixels.iter_mut() {
            *p = pixel;
        }

        // 单色 AND 掩码全 0（完全不透明）。16 像素/行 = 2 字节/行 × 16 行。
        let mask = [0u8; 32];
        let hbm_mask = CreateBitmap(SIZE, SIZE, 1, 1, mask.as_ptr() as *const c_void);
        if hbm_mask == 0 {
            DeleteObject(hbm_color);
            return 0;
        }

        let mut info: ICONINFO = std::mem::zeroed();
        info.fIcon = 1;
        info.hbmColor = hbm_color;
        info.hbmMask = hbm_mask;
        let icon = CreateIconIndirect(&info);
        DeleteObject(hbm_color);
        DeleteObject(hbm_mask);
        icon
    }
}

/// `&str` → 以 NUL 结尾的 UTF-16。
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把 `&str` 复制到固定长度 UTF-16 缓冲（保留 NUL 结尾）。
///
/// 超出缓冲容量的部分被截断；复制结束位置始终写入 NUL，保证短串覆盖长串时
/// 不残留旧尾部（tooltip/气泡短串覆盖长串场景）。
fn copy_wide(dst: &mut [u16], s: &str) {
    if dst.is_empty() {
        return;
    }
    let mut n = 0;
    for unit in s.encode_utf16() {
        if n >= dst.len() {
            break;
        }
        dst[n] = unit;
        n += 1;
    }
    // 在复制结束处写 NUL；n < dst.len() 时覆盖旧尾部首个单元，否则写最后单元。
    dst[dst.len() - 1] = 0;
    if n < dst.len() {
        dst[n] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::copy_wide;

    #[test]
    fn copy_wide_short_string_clears_stale_tail() {
        // 先写入较长字符串，再写较短字符串：旧尾部必须被 NUL 截断，不残留。
        let mut buf = [0u16; 8];
        copy_wide(&mut buf, "abcdef");
        assert_eq!(&buf[..4], &[0x61, 0x62, 0x63, 0x64]);
        assert_eq!(buf[6], 0);

        copy_wide(&mut buf, "xy");
        // 前两个单元为新串，第 3 个单元被 NUL 覆盖——Win32 读到此处即视为终止，
        // 旧尾部不再参与字符串内容（缓冲尾部不必逐单元清零）。
        assert_eq!(&buf[..3], &[0x78, 0x79, 0]);
    }

    #[test]
    fn copy_wide_truncates_and_nul_terminates() {
        let mut buf = [0u16; 3];
        copy_wide(&mut buf, "abcdef");
        // 容量 3：前 2 个单元为字符，最后单元为 NUL（截断）。
        assert_eq!(&buf[..3], &[0x61, 0x62, 0]);
    }

    #[test]
    fn copy_wide_empty_buffer_is_noop() {
        let mut buf: [u16; 0] = [];
        copy_wide(&mut buf, "abc");
    }
}
