//! WebSocket 服务基础设施。
//!
//! - [`serve_with_nodelay`]：数据面与管理面（任务 10）共用的自定义 accept 循环，
//!   每个 `TcpStream` 先 `set_nodelay(true)` 再交给 hyper/axum，以降低小帧透传延迟。
//! - [`data`]：数据面 WebSocket 的握手、attach、二进制透传与 close code。
//! - [`management`]：管理面 WebSocket 的 Origin 校验、`hello`、7 个 action 分发。

pub mod data;
pub mod heartbeat;
pub mod management;

use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;

pub use data::AxumDataListenerFactory;

/// 自定义 accept 循环：每个 `TcpStream` 先 `set_nodelay(true)` 再交给 hyper/axum。
///
/// `shutdown` 触发后停止 accept 并返回。已接受的连接由各自 spawn 的任务独立处理，
/// 其生命周期随运行时终止。管理面（任务 10）复用同一 helper。
pub(crate) async fn serve_with_nodelay(
    listener: TcpListener,
    router: Router,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        // 忽略 set_nodelay 失败：仅影响延迟，不影响功能。
                        let _ = stream.set_nodelay(true);
                        let router = router.clone();
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            // `Router` 实现的是 tower 的 `Service`，这里桥接为 hyper 的 `Service`。
                            let service = TowerToHyperService::new(router.into_service::<Incoming>());
                            let builder = Builder::new(TokioExecutor::new());
                            if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
                                tracing::debug!(error = %err, "WS 连接处理结束");
                            }
                        });
                    }
                    Err(err) => {
                        // 监听错误（如 FD 耗尽）：短暂退避后继续，避免忙等。
                        tracing::warn!(error = %err, "accept 失败");
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }
        }
    }
}
