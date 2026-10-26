//! HTTP 管理页服务与运行时开关。
//!
//! [`AdminService`] 提供可运行时开关的本地 HTTP 管理页（硬编码 `127.0.0.1`），
//! 路由 `/`、`/style.css`、`/js/*`、`/api/config`。它实现 [`AdminController`]，
//! 供管理 WS 的 `admin.set` 同步启停；不持有 [`AppState`]，避免与
//! `AppState.admin` 形成循环——管理 WS 端口与自启状态由构造参数与
//! [`AdminService::set_autostart_status`] 注入。

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::app::AdminController;
use crate::config::AdminConfig;
use crate::protocol::AdminState;

/// 内嵌的首页。
const INDEX_HTML: &str = include_str!("../assets/index.html");
/// 内嵌的样式。
const STYLE_CSS: &str = include_str!("../assets/style.css");
/// 内嵌的 JS 模块（任务 13）。
const JS_API: &str = include_str!("../assets/js/api.js");
const JS_OVERVIEW: &str = include_str!("../assets/js/overview.js");
const JS_ASSISTANT: &str = include_str!("../assets/js/assistant.js");
const JS_MAIN: &str = include_str!("../assets/js/main.js");
const JS_DEMO: &str = include_str!("../assets/js/demo.js");
const JS_FORMAT: &str = include_str!("../assets/js/format.js");
const JS_SERIAL: &str = include_str!("../assets/js/serial.js");

/// 管理页服务：可运行时开关的本地 HTTP 服务。
pub struct AdminService {
    cfg: AdminConfig,
    management_port: u16,
    autostart: Arc<AtomicBool>,
    state: Mutex<Option<AdminHandle>>,
    /// 已解析的监听端口：端口 `0` 时记住系统分配的临时端口，保证开关前后
    /// `url()` 稳定、re-enable 复用同一端口（避免 `AddrInUse`）。
    bound_port: Mutex<u16>,
}

/// 运行中的 HTTP 服务句柄：URL、shutdown 信号与 serve 任务。
struct AdminHandle {
    url: String,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl AdminService {
    /// 只构造、不监听。`port` 为管理页端口（`0` 表示临时端口），
    /// `management_port` 为管理 WS 端口（供 `/api/config` 上报）。
    pub fn new(port: u16, management_port: u16) -> AdminService {
        AdminService {
            cfg: AdminConfig {
                enabled: false,
                port,
            },
            management_port,
            autostart: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(None),
            bound_port: Mutex::new(0),
        }
    }

    /// 构造并启动（测试用）。等价于 [`AdminService::new`] + `set_enabled(true)`。
    pub async fn start(port: u16, management_port: u16) -> io::Result<AdminService> {
        let svc = AdminService::new(port, management_port);
        svc.set_enabled(true).await?;
        Ok(svc)
    }

    /// 运行时启停：`true` 重新绑定并 serve，`false` 触发 graceful shutdown 并
    /// 等待 serve 任务退出（端口真正释放）后返回，保证紧接的 re-enable 不失败。
    pub async fn set_enabled(&self, enabled: bool) -> io::Result<AdminState> {
        if enabled {
            self.bind_and_spawn()
        } else {
            let handle = self.state.lock().unwrap().take();
            if let Some(handle) = handle {
                let _ = handle.shutdown.send(());
                let _ = handle.task.await;
            }
            Ok(AdminState {
                enabled: false,
                url: String::new(),
            })
        }
    }

    /// 当前开关状态。
    pub fn status(&self) -> AdminState {
        match self.state.lock().unwrap().as_ref() {
            Some(handle) => AdminState {
                enabled: true,
                url: handle.url.clone(),
            },
            None => AdminState {
                enabled: false,
                url: String::new(),
            },
        }
    }

    /// 当前管理页 URL；未启用时为空字符串。
    pub fn url(&self) -> String {
        self.status().url
    }

    /// 停止服务（幂等）。
    pub async fn shutdown(&self) {
        let _ = self.set_enabled(false).await;
    }

    /// 更新自启状态（`/api/config` 的 `autostart`；任务 14 接入真实值）。
    pub fn set_autostart_status(&self, enabled: bool) {
        self.autostart.store(enabled, Ordering::SeqCst);
    }

    /// 同步绑定 + spawn 服务任务；已启用时幂等返回当前状态。
    fn bind_and_spawn(&self) -> io::Result<AdminState> {
        let mut guard = self.state.lock().unwrap();
        if let Some(handle) = guard.as_ref() {
            return Ok(AdminState {
                enabled: true,
                url: handle.url.clone(),
            });
        }

        let bind_port = *self.bound_port.lock().unwrap();
        let request_port = if bind_port != 0 {
            bind_port
        } else {
            self.cfg.port
        };

        let listener = std::net::TcpListener::bind(("127.0.0.1", request_port))?;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let resolved = listener.local_addr()?.port();
        *self.bound_port.lock().unwrap() = resolved;

        let url = format!("http://127.0.0.1:{resolved}");
        let router = build_router(
            self.management_port,
            Arc::clone(&self.autostart),
            url.clone(),
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        let state = AdminState {
            enabled: true,
            url: url.clone(),
        };
        *guard = Some(AdminHandle {
            url,
            shutdown: shutdown_tx,
            task,
        });
        Ok(state)
    }
}

impl AdminController for AdminService {
    fn status(&self) -> AdminState {
        AdminService::status(self)
    }

    fn autostart(&self) -> bool {
        self.autostart.load(Ordering::SeqCst)
    }

    fn set_enabled(&self, enabled: bool) -> Pin<Box<dyn Future<Output = AdminState> + Send + '_>> {
        Box::pin(async move {
            match AdminService::set_enabled(self, enabled).await {
                Ok(state) => state,
                Err(err) => {
                    tracing::warn!(error = %err, "管理页启动失败");
                    AdminState {
                        enabled: false,
                        url: String::new(),
                    }
                }
            }
        })
    }
}

/// `/api/config` 响应：`{managementPort, version, admin:{enabled,url}, autostart}`。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigResponse {
    management_port: u16,
    version: String,
    admin: AdminState,
    autostart: bool,
}

/// 路由共享状态。
struct HttpState {
    management_port: u16,
    url: String,
    autostart: Arc<AtomicBool>,
}

fn build_router(management_port: u16, autostart: Arc<AtomicBool>, url: String) -> Router {
    let state = Arc::new(HttpState {
        management_port,
        url,
        autostart,
    });
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(style))
        .route("/js/*path", get(js))
        .route("/api/config", get(config))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

/// 静态分发 `/js/*`：按文件名返回内嵌 JS 模块，未知路径 404。
async fn js(Path(path): Path<String>) -> Result<impl IntoResponse, StatusCode> {
    let (content_type, body): (&str, &'static str) = match path.as_str() {
        "api.js" => ("text/javascript; charset=utf-8", JS_API),
        "overview.js" => ("text/javascript; charset=utf-8", JS_OVERVIEW),
        "assistant.js" => ("text/javascript; charset=utf-8", JS_ASSISTANT),
        "main.js" => ("text/javascript; charset=utf-8", JS_MAIN),
        "demo.js" => ("text/javascript; charset=utf-8", JS_DEMO),
        "format.js" => ("text/javascript; charset=utf-8", JS_FORMAT),
        "serial.js" => ("text/javascript; charset=utf-8", JS_SERIAL),
        _ => return Err(StatusCode::NOT_FOUND),
    };
    Ok(([(header::CONTENT_TYPE, content_type)], body))
}

async fn config(State(state): State<Arc<HttpState>>) -> Json<ConfigResponse> {
    Json(ConfigResponse {
        management_port: state.management_port,
        version: env!("CARGO_PKG_VERSION").to_string(),
        admin: AdminState {
            enabled: true,
            url: state.url.clone(),
        },
        autostart: state.autostart.load(Ordering::SeqCst),
    })
}
