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
        "ports",
        "sessions",
        "admin-toggle",
        "export",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    assert!(!html.contains("cdn."));
    assert!(!html.contains("https://unpkg"));
    let api = include_str!("../src/assets/js/api.js");
    assert!(api.contains("127.0.0.1"));
    assert!(api.contains("reconnect"));
}

/// `/js/*` 由 HTTP 管理服务静态分发，且内容非空、Content-Type 正确。
#[tokio::test]
async fn serves_js_assets() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    for path in ["api.js", "overview.js", "assistant.js", "main.js"] {
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
