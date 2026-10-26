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
    for id in [
        "overview",
        "assistant",
        "main-tabs",
        "demo",
        "ports",
        "sessions",
        "admin-toggle",
        "export",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    for id in [
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
    assert!(!html.contains("id=\"tab-"));
    assert!(!html.contains("cdn."));
    assert!(!html.contains("https://unpkg"));
    let api = include_str!("../src/assets/js/api.js");
    assert!(api.contains("127.0.0.1"));
    assert!(api.contains("reconnect"));
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

/// 管理页把演示 DOM 并进同一文档后，所有 id 必须唯一。
#[test]
fn admin_html_ids_are_unique() {
    let html = include_str!("../src/assets/index.html");
    let mut ids = collect_ids(html);
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "index.html 存在重复 id");
}

/// 演示模块必须使用共享的 ManagementClient 契约（request 直接返回 data）。
#[test]
fn demo_module_uses_shared_client_contract() {
    let js = include_str!("../src/assets/js/demo.js");
    assert!(js.contains("function renderTerminal"), "缺少终端渲染实现");
    assert!(js.contains("function openSlot"), "缺少会话打开实现");
    assert!(!js.contains("class ManagementClient"), "演示模块不得自带管理客户端");
    assert!(!js.contains("r.data"), "request() 已直接返回 data");
}

/// `/js/*` 由 HTTP 管理服务静态分发，且内容非空、Content-Type 正确。
#[tokio::test]
async fn serves_js_assets() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    for path in ["api.js", "overview.js", "assistant.js", "main.js", "demo.js"] {
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
