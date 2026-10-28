//! 桌面窗口状态与 WebView 导航策略（不依赖 Win32，便于测试）。

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEvent {
    Show,
    Minimize,
    Close,
    AdminEnabled(bool),
    Quit,
}

#[derive(Debug, Default)]
pub struct DesktopState {
    visible: bool,
    quitting: bool,
}

impl DesktopState {
    pub fn reduce(&mut self, event: DesktopEvent) {
        match event {
            DesktopEvent::Show => {
                if !self.quitting {
                    self.visible = true;
                }
            }
            DesktopEvent::Minimize | DesktopEvent::Close => self.visible = false,
            DesktopEvent::AdminEnabled(enabled) => {
                if !enabled {
                    self.visible = false;
                }
            }
            DesktopEvent::Quit => {
                self.visible = false;
                self.quitting = true;
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting
    }
}

/// URL 未加载或变更时才导航；仅恢复窗口不应重载页面。
pub fn should_navigate(loaded_url: Option<&str>, target: &str) -> bool {
    loaded_url != Some(target)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationAction {
    Allow,
    OpenExternal,
    Block,
}

/// 主框架仅允许留在管理页的回环源；其他 HTTP(S) 地址交系统浏览器。
pub fn navigation_action(admin_url: &str, target: &str) -> NavigationAction {
    let Ok(target) = Url::parse(target) else {
        return NavigationAction::Block;
    };
    if !matches!(target.scheme(), "http" | "https") {
        return NavigationAction::Block;
    }
    let Ok(admin) = Url::parse(admin_url) else {
        return NavigationAction::OpenExternal;
    };
    if same_origin(&admin, &target) && is_loopback(&admin) {
        NavigationAction::Allow
    } else {
        NavigationAction::OpenExternal
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}
