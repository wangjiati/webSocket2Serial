//! 管理页静态资源：必要的 DOM 控制点、无外部 CDN、JS 固定回环地址与重连逻辑，
//! 以及 `/js/*` 静态分发。

use websocket2serial::http::AdminService;

/// 本机 HTTP 客户端。固定 `no_proxy()`，避免开发机系统代理拦截 `127.0.0.1` 请求。
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("构建 reqwest 客户端失败")
}

#[test]
fn admin_assets_have_required_controls_and_no_cdn() {
    let html = include_str!("../src/assets/index.html");
    for id in ["overview", "ports", "sessions", "admin-toggle", "copy-url"] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
}

/// 管理页入口链接必须在新标签打开。
#[test]
fn index_page_links_open_new_tabs() {
    let html = include_str!("../src/assets/index.html");
    for href in ["/assistant", "/test"] {
        assert!(html.contains(&format!("href=\"{href}\"")), "missing {href}");
    }
    assert!(html.contains("target=\"_blank\""));
    assert!(!html.contains("main-tabs"));
    assert!(!html.contains("id=\"demo\""));
    assert!(!html.contains("cdn."));
    assert!(!html.contains("https://unpkg"));
    let api = include_str!("../src/assets/js/api.js");
    assert!(api.contains("127.0.0.1"));
    assert!(api.contains("reconnect"));
}

/// 功能测试页保留现有演示 DOM 与 7 个子 tab。
#[test]
fn test_page_has_demo_controls() {
    let html = include_str!("../src/assets/test.html");
    for id in [
        "demo",
        "demo-statusbar",
        "demo-tabs",
        "demo-tab-overview",
        "demo-tab-ports",
        "demo-tab-sessions",
        "demo-tab-terminal",
        "demo-tab-debug",
        "demo-tab-edge",
        "demo-tab-events",
        "demo-btn-run-demo",
        "demo-ports-body",
        "demo-sessions-body",
        "demo-log-A",
        "demo-log-B",
        "demo-debug-body",
        "demo-edge-log",
        "demo-event-log",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    for tab in [
        "overview", "ports", "sessions", "terminal", "debug", "edge", "events",
    ] {
        assert!(
            html.contains(&format!("data-demo-tab=\"{tab}\"")),
            "missing tab {tab}"
        );
    }
    assert!(!html.contains("cdn."));
}

/// 提取 HTML 中所有 `id="..."`，用于检查重复 ID。
fn collect_ids(html: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("id=\"") {
        let start = pos + 4;
        let end = start + rest[start..].find('"').unwrap();
        ids.push(rest[start..end].to_string());
        rest = &rest[end..];
    }
    ids
}

/// 三份 HTML 的 id 分别唯一。
#[test]
fn admin_html_ids_are_unique() {
    for (label, html) in [
        ("index.html", include_str!("../src/assets/index.html")),
        (
            "assistant.html",
            include_str!("../src/assets/assistant.html"),
        ),
        ("test.html", include_str!("../src/assets/test.html")),
    ] {
        let mut ids = collect_ids(html);
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "{label} 存在重复 id");
    }
}

/// 助手页：固定提示、连接/收发/导出控制点。
#[test]
fn assistant_page_has_notice_and_controls() {
    let html = include_str!("../src/assets/assistant.html");
    for id in [
        "assistant-page",
        "assistant-notice",
        "assistant-port",
        "assistant-port-list",
        "assistant-baud",
        "assistant-databits",
        "assistant-parity",
        "assistant-stopbits",
        "assistant-flowcontrol",
        "assistant-dtr",
        "assistant-rts",
        "assistant-open",
        "assistant-close",
        "assistant-open-result",
        "assistant-counters",
        "assistant-last-error",
        "assistant-state",
        "assistant-log",
        "assistant-pause",
        "assistant-clear",
        "assistant-autoscroll",
        "assistant-ts",
        "assistant-format",
        "assistant-export",
        "assistant-buffer-note",
        "assistant-send-mode",
        "assistant-send-eol",
        "assistant-send-input",
        "assistant-send-btn",
        "assistant-send-status",
        "assistant-encoding",
        "assistant-history",
        "assistant-checksum",
        "assistant-timer-interval",
        "assistant-timer-count",
        "assistant-timer-start",
        "assistant-timer-stop",
        "assistant-timer-status",
        "assistant-cmd-name",
        "assistant-cmd-payload",
        "assistant-cmd-mode",
        "assistant-cmd-eol",
        "assistant-cmd-save",
        "assistant-cmd-list",
        "assistant-rx",
        "assistant-tx",
        "assistant-rate",
        "assistant-drop",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    assert!(html.contains("WebSocket2Serial"));
    assert!(html.contains("Web Serial"));
    assert!(html.contains("PORT_BUSY"));
    assert!(!html.contains("cdn."));
}

/// `/assistant` 返回独立页面。
#[tokio::test]
async fn serves_assistant_page() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let resp = client()
        .get(format!("{}/assistant", svc.url()))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "/assistant 未返回成功");
    let html = resp.text().await.unwrap();
    assert!(html.contains("串口助手"));
    svc.shutdown().await;
}

/// `/test` 返回独立功能测试页。
#[tokio::test]
async fn serves_test_page() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let resp = client()
        .get(format!("{}/test", svc.url()))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "/test 未返回成功");
    let html = resp.text().await.unwrap();
    assert!(html.contains("功能测试"));
    svc.shutdown().await;
}

/// 演示模块必须使用共享的 ManagementClient 契约（request 直接返回 data）。
#[test]
fn demo_module_uses_shared_client_contract() {
    let js = include_str!("../src/assets/js/demo.js");
    assert!(js.contains("function renderTerminal"), "缺少终端渲染实现");
    assert!(js.contains("function openSlot"), "缺少会话打开实现");
    assert!(
        js.contains("from \"./format.js\""),
        "demo.js 应复用 format.js"
    );
    assert!(
        js.contains("from \"./serial.js\""),
        "demo.js 应复用 serial.js"
    );
    assert!(
        !js.contains("class SessionController"),
        "会话控制器应移到 serial.js"
    );
    assert!(
        !js.contains("class ManagementClient"),
        "演示模块不得自带管理客户端"
    );
    assert!(!js.contains("r.data"), "request() 已直接返回 data");
}

/// `/js/*` 由 HTTP 管理服务静态分发，且内容非空、Content-Type 正确。
#[tokio::test]
async fn serves_js_assets() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    for path in [
        "api.js",
        "overview.js",
        "main.js",
        "demo.js",
        "format.js",
        "serial.js",
        "assistant-page.js",
        "test-page.js",
    ] {
        let resp = client()
            .get(format!("{}/js/{path}", svc.url()))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "/js/{path} 未返回成功");
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            content_type.contains("javascript"),
            "/js/{path} Content-Type 非 javascript：{content_type}"
        );
        let body = resp.text().await.unwrap();
        assert!(!body.is_empty(), "/js/{path} 返回空内容");
    }
    svc.shutdown().await;
}
