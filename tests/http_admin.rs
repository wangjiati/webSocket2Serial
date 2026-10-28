//! HTTP 管理页服务：`/api/config`、静态首页、运行时开关与 `AdminController` 接线。

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use websocket2serial::app::{ensure_admin_url, AdminController};
use websocket2serial::http::AdminService;

/// 本机 HTTP 客户端。固定 `no_proxy()`，避免开发机系统代理（如 `127.0.0.1:10808`）
/// 拦截 `127.0.0.1` 的请求、把「连接被拒」变成代理返回的 503。
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("构建 reqwest 客户端失败")
}

#[tokio::test]
async fn serves_config_and_index() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let cfg: Value = client()
        .get(format!("{}/api/config", svc.url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg["managementPort"], 8765);
    let html = client()
        .get(svc.url())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("WebSocket2Serial"));
    svc.shutdown().await;
}

#[tokio::test]
async fn admin_can_be_disabled_and_reenabled() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let url = svc.url().to_string();
    svc.set_enabled(false).await.unwrap();
    assert!(client().get(&url).send().await.is_err());
    svc.set_enabled(true).await.unwrap();
    assert!(client()
        .get(&url)
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    svc.shutdown().await;
}

/// 管理 WS 实际走 `Arc<dyn AdminController>` 的异步 `set_enabled`；这里直接以 trait
/// 对象驱动真实 HTTP 服务，验证异步开关（含关→开）能正确释放并重新绑定端口。
#[tokio::test]
async fn admin_controller_trait_drives_http_service() {
    let svc = AdminService::new(0, 8765);
    let controller: Arc<dyn AdminController> = Arc::new(svc);

    let enabled = controller.set_enabled(true).await;
    assert!(enabled.enabled);
    wait_reachable(&enabled.url).await;

    let disabled = controller.set_enabled(false).await;
    assert!(!disabled.enabled);
    wait_unreachable(&enabled.url).await;

    let reenabled = controller.set_enabled(true).await;
    assert!(reenabled.enabled);
    wait_reachable(&reenabled.url).await;

    controller.set_enabled(false).await;
}

#[tokio::test]
async fn ensure_admin_url_reopens_a_disabled_service() {
    let svc = AdminService::new(0, 8765);
    assert!(!svc.status().enabled);
    let controller: Arc<dyn AdminController> = Arc::new(svc);
    let url = ensure_admin_url(controller.as_ref())
        .await
        .expect("应从真实状态重新启用管理页");
    wait_reachable(&url).await;
    controller.set_enabled(false).await;
}

/// 关→立即开不得出现 `AddrInUse`：`set_enabled(false)` 必须 await 到 serve 任务退出、
/// 端口真正释放后才返回，因此紧接的 `set_enabled(true)` 必须成功且立即可访问。
#[tokio::test]
async fn immediate_disable_then_enable_via_controller() {
    let svc = AdminService::new(0, 8765);
    let controller: Arc<dyn AdminController> = Arc::new(svc);

    let enabled = controller.set_enabled(true).await;
    assert!(enabled.enabled);
    wait_reachable(&enabled.url).await;

    // 不轮询等待端口释放，连续 disable 后立刻 enable。
    let disabled = controller.set_enabled(false).await;
    assert!(!disabled.enabled);
    let reenabled = controller.set_enabled(true).await;
    assert!(reenabled.enabled);

    assert!(client()
        .get(format!("{}/api/config", reenabled.url))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());

    controller.set_enabled(false).await;
}

async fn wait_reachable(url: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if client()
            .get(url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "HTTP 服务未在 5s 内可达：{url}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_unreachable(url: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if client().get(url).send().await.is_err() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "HTTP 服务未在 5s 内停止：{url}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
