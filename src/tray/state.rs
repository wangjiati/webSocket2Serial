//! 托盘状态 → 图标颜色 / 提示文本的纯函数。

use crate::protocol::SessionState;

/// `Shell_NotifyIcon` v4 回调中的用户动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMouseAction {
    OpenAdmin,
    OpenMenu,
    None,
}

/// 解析 `NOTIFYICON_VERSION_4` 的 `lParam`：LOWORD 为事件，HIWORD 为图标 ID。
///
/// v4 同时使用 `NIN_SELECT`/`NIN_KEYSELECT` 表示激活，不能只比较旧版 WM_* 值。
pub fn tray_mouse_action(lparam: isize) -> TrayMouseAction {
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const NIN_SELECT: u32 = 0x0400;
    const NIN_KEYSELECT: u32 = 0x0401;

    match (lparam as u32) & 0xFFFF {
        WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => TrayMouseAction::OpenAdmin,
        WM_RBUTTONUP | WM_CONTEXTMENU => TrayMouseAction::OpenMenu,
        _ => TrayMouseAction::None,
    }
}

/// 托盘图标状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    /// 无会话（灰）。
    Idle,
    /// 至少一个 active（绿）。
    Active,
    /// 有 awaiting 或 grace（黄）。
    Waiting,
    /// 最近一次会话因错误关闭（红）。
    Error,
}

impl TrayStatus {
    /// 状态点颜色（0x00RRGGBB）。
    pub fn color(self) -> u32 {
        match self {
            TrayStatus::Idle => 0x00808080,
            TrayStatus::Active => 0x0000A000,
            TrayStatus::Waiting => 0x00D0A000,
            TrayStatus::Error => 0x00C00000,
        }
    }

    /// 菜单状态行的简短中文标签。
    pub fn label(self) -> &'static str {
        match self {
            TrayStatus::Idle => "空闲",
            TrayStatus::Active => "运行中",
            TrayStatus::Waiting => "等待连接",
            TrayStatus::Error => "错误",
        }
    }
}

/// 从会话状态集合与最近错误计算托盘状态。
///
/// 优先级：错误 > active > awaiting/grace > 空闲。`last_error` 为最近一次因
/// 错误关闭的会话 reason（如 `DEVICE_LOST`）；存在即显示红色。
pub fn status_for(sessions: &[SessionState], last_error: Option<&str>) -> TrayStatus {
    if last_error.is_some() {
        return TrayStatus::Error;
    }
    if sessions.iter().any(|s| *s == SessionState::Active) {
        return TrayStatus::Active;
    }
    if sessions
        .iter()
        .any(|s| matches!(*s, SessionState::Awaiting | SessionState::Grace))
    {
        return TrayStatus::Waiting;
    }
    TrayStatus::Idle
}

/// 托盘 tooltip 文本：版本、管理端口、会话数、最近错误。
pub fn tooltip_text(
    version: &str,
    management_port: u16,
    session_count: usize,
    last_error: Option<&str>,
) -> String {
    let mut text =
        format!("WebSocket2Serial v{version} · 端口 {management_port} · {session_count} 个会话");
    if let Some(err) = last_error {
        text.push_str(&format!(" · 最近错误: {err}"));
    }
    text
}
