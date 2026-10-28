//! 管理页静态资源：必要的 DOM 控制点、无外部 CDN、JS 固定回环地址与重连逻辑，
//! 以及 `/js/*` 静态分发。

use std::process::Command;

use websocket2serial::http::AdminService;

/// 使用仓库已有的可选 Node 环境运行真实管理页 JS 行为测试。
fn run_assistant_js_behavior(case: &str) {
    let output = match Command::new("node")
        .args(["tests/js/assistant-behavior.test.mjs", case])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            eprintln!("跳过：无法执行 node（{err}）");
            return;
        }
    };
    assert!(
        output.status.success(),
        "JS 行为用例 {case} 失败\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 管理/数据 WS trace 必须保留原文、关联乱序回应并隔离观察回调异常。
#[test]
fn websocket_trace_hooks_capture_raw_traffic_safely() {
    run_assistant_js_behavior("trace");
}

/// `ports.list` 刷新必须向调用方报告结果，失败时保留旧快照。
#[test]
fn management_refresh_ports_reports_success() {
    run_assistant_js_behavior("refresh");
}

/// 助手页提供手动重扫串口入口。
#[test]
fn assistant_can_refresh_ports_on_demand() {
    let html = include_str!("../src/assets/assistant.html");
    assert!(html.contains("id=\"assistant-refresh-ports\""));
    assert!(html.contains(">刷新串口</button>"));
}

/// WS 报文面板提供完整控制点，默认收起并链接协议帮助。
#[test]
fn assistant_has_chronological_ws_trace_panel() {
    let html = include_str!("../src/assets/assistant.html");
    for id in [
        "assistant-ws-trace",
        "assistant-ws-trace-ts",
        "assistant-ws-trace-pause",
        "assistant-ws-trace-clear",
        "assistant-ws-trace-export",
        "assistant-ws-trace-note",
        "assistant-ws-trace-log",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    assert!(html.contains("<details id=\"assistant-ws-trace\" class=\"card assistant-trace\">"));
    let panel = html
        .split("<details id=\"assistant-ws-trace\"")
        .nth(1)
        .unwrap();
    assert!(panel.contains("观察管理 WS 与数据 WS 的报文原文"));
    assert!(panel.contains("href=\"/help\""));
}

/// 面板使用的有界缓冲必须在第 2001 条到来时丢弃最旧记录。
#[test]
fn assistant_trace_buffer_is_capped() {
    run_assistant_js_behavior("panel");
}

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
    for href in ["/assistant", "/test", "/help"] {
        assert!(html.contains(&format!("href=\"{href}\"")), "missing {href}");
    }
    assert!(html.contains("target=\"_blank\""));
    assert!(!html.contains("main-tabs"));
    assert!(!html.contains("id=\"demo\""));
    assert!(!html.contains("entry-assistant"));
    assert!(!html.contains("entry-test"));
    assert!(!html.contains("cdn."));
    assert!(!html.contains("https://unpkg"));
    let api = include_str!("../src/assets/js/api.js");
    assert!(api.contains("127.0.0.1"));
    assert!(api.contains("reconnect"));
}

/// 帮助文档独立页：包含通信协议章节。
#[test]
fn help_page_has_protocol_chapter() {
    let html = include_str!("../src/assets/help.html");
    assert!(html.contains("id=\"help-page\""));
    assert!(html.contains("通信协议"));
    for term in [
        "ws2s/v1",
        "ports.list",
        "session.open",
        "session.close",
        "session.list",
        "session.write",
        "debug.subscribe",
        "admin.set",
        "hello",
        "ports.changed",
        "session.state",
        "debug.frame",
        "admin.state",
        "PORT_BUSY",
        "4002",
        "4004",
        "1009",
        "ws://127.0.0.1:8765/ws",
    ] {
        assert!(html.contains(term), "帮助页缺少协议内容：{term}");
    }
    assert!(!html.contains("cdn."));
}

/// 管理页自启状态、会话 DROP 与帮助入口。
#[test]
fn admin_help_and_monitor_controls() {
    let html = include_str!("../src/assets/index.html");
    assert!(html.contains("id=\"autostart\""));
    assert!(html.contains("href=\"/help\""), "missing help link");
    assert!(html.contains("<th>DROP</th>"));
    assert!(!html.contains("id=\"help\""), "帮助文档应独立成页");
    let js = include_str!("../src/assets/js/overview.js");
    assert!(js.contains("/assistant?port="), "端口行应能打开助手");
    assert!(js.contains("refreshSessions"), "管理页应轮询会话");
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
        ("help.html", include_str!("../src/assets/help.html")),
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

/// `/help` 返回独立帮助文档页。
#[tokio::test]
async fn serves_help_page() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let resp = client()
        .get(format!("{}/help", svc.url()))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "/help 未返回成功");
    let html = resp.text().await.unwrap();
    assert!(html.contains("帮助文档"));
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

/// 所有管理页资源禁用浏览器缓存，避免升级 exe 后继续用旧页面/旧模块。
#[tokio::test]
async fn static_assets_are_no_store() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    for path in [
        "/",
        "/assistant",
        "/test",
        "/help",
        "/style.css",
        "/js/api.js",
        "/api/config",
    ] {
        let resp = client()
            .get(format!("{}{}", svc.url(), path))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{path} 未返回成功");
        let cache = resp
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            cache.contains("no-store"),
            "{path} 缺少 Cache-Control: no-store（实际：{cache}）"
        );
    }
    svc.shutdown().await;
}

/// 旧的监控助手模块已删除。
#[tokio::test]
async fn removed_assistant_js_returns_404() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let resp = client()
        .get(format!("{}/js/assistant.js", svc.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    svc.shutdown().await;
}

/// 助手统计必须使用服务端权威 counters，而不是会被 4 MiB 上限裁剪的 frames。
#[test]
fn assistant_uses_authoritative_counters() {
    let js = include_str!("../src/assets/js/assistant-page.js");
    assert!(js.contains("counters.rxBytes"), "RX 应取服务端 counters");
    assert!(js.contains("counters.txBytes"), "TX 应取服务端 counters");
    assert!(
        js.contains("client.refreshSessions"),
        "助手应定期刷新会话计数"
    );
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
