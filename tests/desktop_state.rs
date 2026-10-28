use websocket2serial::desktop::state::{
    navigation_action, DesktopEvent, DesktopState, NavigationAction,
};

#[test]
fn minimize_and_close_hide_without_quitting() {
    for event in [DesktopEvent::Minimize, DesktopEvent::Close] {
        let mut state = DesktopState::default();
        state.reduce(DesktopEvent::Show);
        state.reduce(event);
        assert!(!state.is_visible());
        assert!(!state.is_quitting());
    }
}

#[test]
fn show_restores_a_hidden_window() {
    let mut state = DesktopState::default();
    assert!(!state.is_visible());
    state.reduce(DesktopEvent::Show);
    assert!(state.is_visible());
    state.reduce(DesktopEvent::Minimize);
    state.reduce(DesktopEvent::Show);
    assert!(state.is_visible());
}

#[test]
fn quit_is_distinct_from_close() {
    let mut state = DesktopState::default();
    state.reduce(DesktopEvent::Show);
    state.reduce(DesktopEvent::Quit);
    assert!(!state.is_visible());
    assert!(state.is_quitting());
}

#[test]
fn disabling_admin_hides_the_window() {
    let mut state = DesktopState::default();
    state.reduce(DesktopEvent::Show);
    state.reduce(DesktopEvent::AdminEnabled(false));
    assert!(!state.is_visible());
    assert!(!state.is_quitting());
}

#[test]
fn navigation_allows_only_the_configured_loopback_origin() {
    let admin = "http://127.0.0.1:8766/";
    assert_eq!(
        navigation_action(admin, "http://127.0.0.1:8766/assistant?port=COM3"),
        NavigationAction::Allow
    );
    assert_eq!(
        navigation_action(admin, "http://127.0.0.1:8766/js/main.js"),
        NavigationAction::Allow
    );
    assert_eq!(
        navigation_action(admin, "http://127.0.0.1:8765/"),
        NavigationAction::OpenExternal
    );
    assert_eq!(
        navigation_action(admin, "http://127.0.0.1.evil.example/"),
        NavigationAction::OpenExternal
    );
    assert_eq!(
        navigation_action(admin, "https://example.com/"),
        NavigationAction::OpenExternal
    );
}

#[test]
fn navigation_blocks_non_web_and_malformed_urls() {
    let admin = "http://127.0.0.1:8766/";
    for target in ["file:///C:/Windows/system.ini", "javascript:alert(1)", "not a url"] {
        assert_eq!(
            navigation_action(admin, target),
            NavigationAction::Block,
            "target: {target}"
        );
    }
}
