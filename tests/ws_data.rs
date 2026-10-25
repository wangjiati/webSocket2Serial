//! 数据 WebSocket：握手、二进制回显、文本/超大帧 close code、第二客户端 4002、
//! 正常 close 释放端口与 TX 洪泛不丢包。

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{sample_open, FakeTransport, FakeTransportFactory};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use websocket2serial::config::{OriginAllowlist, SessionConfig};
use websocket2serial::protocol::SessionState;
use websocket2serial::serial::TransportError;
use websocket2serial::session::{CloseReason, SessionManager};
use websocket2serial::ws::AxumDataListenerFactory;

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// 测试应用：真实数据监听器工厂（`Arc::new_cyclic` 传入 manager 弱引用）+ 假串口。
struct App {
    manager: Arc<SessionManager>,
    serial: FakeTransport,
}

async fn test_app() -> App {
    test_app_with_config(SessionConfig::default()).await
}

/// 指定宽限时长的测试应用（`grace_timeout_closes_session` 用 100ms 缩短宽限）。
async fn test_app_with_grace(grace_ms: u64) -> App {
    test_app_with_config(SessionConfig {
        grace_ms,
        ..SessionConfig::default()
    })
    .await
}

/// rx 通道容量最小的测试应用（背压用例：`rx_queue_bytes=4096` → 容量 1）。
async fn test_app_with_rx_queue(rx_queue_bytes: usize) -> App {
    test_app_with_config(SessionConfig {
        rx_queue_bytes,
        ..SessionConfig::default()
    })
    .await
}

async fn test_app_with_config(cfg: SessionConfig) -> App {
    let serial = FakeTransport::new();
    let transport_factory = Arc::new(FakeTransportFactory::with_transport(serial.clone()));
    let origins = OriginAllowlist::new(&[
        "http://127.0.0.1:*".to_string(),
        "http://localhost:*".to_string(),
    ]);
    let (events, _) = broadcast::channel(16);
    let manager = Arc::new_cyclic(|weak| {
        let factory = Arc::new(AxumDataListenerFactory::new(
            weak.clone(),
            origins,
            cfg.tx_queue_bytes,
        ));
        SessionManager::new(transport_factory, factory, cfg, events)
    });
    App { manager, serial }
}

/// 连接数据 WS，携带允许的 Origin（tungstenite 默认不发 Origin，会被拒绝）。
async fn connect_data(url: &str) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "origin",
        http::HeaderValue::from_static("http://127.0.0.1:1234"),
    );
    let (ws, _response) = connect_async(request).await?;
    Ok(ws)
}

/// 读取下一条二进制帧（跳过 ping/pong/close 之外的控制帧）。
async fn next_binary(ws: &mut Ws) -> Vec<u8> {
    loop {
        match ws.next().await.expect("流结束").expect("读取错误") {
            Message::Binary(data) => return data,
            _ => continue,
        }
    }
}

/// 读取关闭帧的 close code；无 code 视为 1000。
async fn close_code(ws: &mut Ws) -> u16 {
    loop {
        match ws.next().await.expect("流结束").expect("读取错误") {
            Message::Close(Some(frame)) => return u16::from(frame.code),
            Message::Close(None) => return 1000,
            _ => continue,
        }
    }
}

/// 打开一个会话并建立数据 WS（attach 进入 active），返回 (连接, data_url, 应用)。
async fn open_active_data_ws() -> (Ws, String, App) {
    let app = test_app().await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let url = open.data_url.clone();
    let ws = connect_data(&url).await.unwrap();
    (ws, url, app)
}

/// 短轮询等待会话进入指定状态（用 `session.list` 而非固定 sleep，避免时序竞态）。
async fn wait_state(app: &App, expected: SessionState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let current = app.manager.list().await.first().map(|i| i.state);
        if current == Some(expected) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待状态 {expected:?} 超时"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn data_ws_echoes_binary() {
    let app = test_app().await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let mut ws = connect_data(&open.data_url).await.unwrap();
    ws.send(Message::Binary(b"ping".to_vec())).await.unwrap();
    app.serial.expect_tx(b"ping").await;
    app.serial.push_rx(b"pong");
    assert_eq!(next_binary(&mut ws).await, b"pong");
}

#[tokio::test]
async fn text_frame_closes_1003() {
    let (mut ws, _url, _app) = open_active_data_ws().await;
    ws.send(Message::Text("hello".into())).await.unwrap();
    assert_eq!(close_code(&mut ws).await, 1003);
}

#[tokio::test]
async fn wrong_token_returns_404() {
    let app = test_app().await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let bad = open.data_url.replace("/ws/", "/ws/bad");
    assert!(connect_data(&bad).await.is_err());
}

#[tokio::test]
async fn second_client_is_closed_4002() {
    let (mut first, url, _app) = open_active_data_ws().await;
    let mut second = connect_data(&url).await.unwrap();
    assert_eq!(close_code(&mut second).await, 4002);
    // 第一个客户端不受影响，仍可发送。
    first.send(Message::Binary(b"ok".to_vec())).await.unwrap();
}

#[tokio::test]
async fn oversized_frame_closes_1009() {
    let (mut ws, _url, _app) = open_active_data_ws().await;
    ws.send(Message::Binary(vec![0u8; 1024 * 1024 + 1]))
        .await
        .unwrap();
    assert_eq!(close_code(&mut ws).await, 1009);
}

#[tokio::test]
async fn device_loss_sends_4001() {
    let (mut ws, _url, app) = open_active_data_ws().await;
    app.serial.fail_next_read(TransportError::DeviceLost);
    assert_eq!(close_code(&mut ws).await, 4001);
}

#[tokio::test]
async fn normal_close_releases_port() {
    let (mut ws, _url, app) = open_active_data_ws().await;
    ws.close(None).await.unwrap();

    // 等待会话清理：`list()` 先变空，端口锁随后释放，轮询 reopen 直至成功。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let again = loop {
        match app.manager.open(sample_open("COM3")).await {
            Ok(again) => break again,
            Err(websocket2serial::session::ManagerError::PortBusy) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "正常 close 未释放端口"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(other) => panic!("意外错误: {other:?}"),
        }
    };
    assert!(!again.reused);
}

#[tokio::test]
async fn tx_flood_preserves_order_and_completeness() {
    let app = test_app().await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let mut ws = connect_data(&open.data_url).await.unwrap();

    // 多帧连续写入：容量 1 的 tx 通道 + `send().await` 背压，不允许静默丢 TX。
    let frames: Vec<Vec<u8>> = (0..50)
        .map(|i| format!("frame-{i:04}").into_bytes())
        .collect();
    let expected: Vec<u8> = frames.iter().flatten().copied().collect();

    for frame in &frames {
        ws.send(Message::Binary(frame.clone())).await.unwrap();
    }

    // 断言写入顺序与字节数完整。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let written = app.serial.written();
        if written == expected {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "TX 洪泛丢失或乱序：已写 {} 字节，期望 {} 字节",
            written.len(),
            expected.len()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn abnormal_drop_enters_grace_and_reattach_reports_gap() {
    let app = test_app().await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let url = open.data_url.clone();
    let mut ws = connect_data(&url).await.unwrap();

    app.serial.push_rx(b"a");
    assert_eq!(next_binary(&mut ws).await, b"a");

    drop(ws); // 非 1000 断开 → 进入宽限
    wait_state(&app, SessionState::Grace).await;

    app.serial.push_rx(b"b"); // 宽限内读到但丢弃
    app.serial.push_rx(b"c");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let discarded = app.manager.list().await[0].counters.rx_discarded_bytes;
        if discarded >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "宽限丢弃计数未增长：{discarded}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 重连旧 dataUrl：resume 并写入 lastGap。
    let mut ws2 = connect_data(&url).await.unwrap();
    let state = loop {
        let state = app.manager.list().await[0].clone();
        if state.last_gap.is_some() {
            break state;
        }
        assert!(tokio::time::Instant::now() < deadline, "last_gap 未写入");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let gap = state.last_gap.unwrap();
    assert_eq!(gap.discarded_bytes, 2);
    assert!(gap.resumed);

    // 重连后只收新数据，不收宽限期的积压旧数据。
    app.serial.push_rx(b"d");
    assert_eq!(next_binary(&mut ws2).await, b"d");
}

#[tokio::test]
async fn grace_timeout_closes_session() {
    let app = test_app_with_grace(100).await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let ws = connect_data(&open.data_url).await.unwrap();
    drop(ws); // 非 1000 断开 → 进入宽限

    wait_state(&app, SessionState::Grace).await;

    // 100ms 宽限超时后会话销毁、端口释放。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !app.manager.list().await.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "宽限超时未销毁会话");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // `list()` 先变空，端口锁随后释放：轮询 reopen 直至成功。
    let again = loop {
        match app.manager.open(sample_open("COM3")).await {
            Ok(again) => break again,
            Err(websocket2serial::session::ManagerError::PortBusy) => {
                assert!(tokio::time::Instant::now() < deadline, "宽限超时未释放端口");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(other) => panic!("意外错误: {other:?}"),
        }
    };
    assert!(!again.reused);
}

#[tokio::test]
async fn admin_close_sends_4004() {
    let (mut ws, _url, app) = open_active_data_ws().await;
    let id = app.manager.list().await[0].session_id.clone();
    app.manager
        .close(&id, CloseReason::AdminClosed)
        .await
        .unwrap();
    assert_eq!(close_code(&mut ws).await, 4004);
}

#[tokio::test]
async fn write_stall_sends_4007() {
    let (mut ws, _url, app) = open_active_data_ws().await;
    app.serial.fail_next_write(TransportError::WriteTimeout);
    ws.send(Message::Binary(b"x".to_vec())).await.unwrap();
    assert_eq!(close_code(&mut ws).await, 4007);
}

#[tokio::test]
async fn backpressure_sends_4005() {
    let app = test_app_with_rx_queue(4096).await;
    let open = app.manager.open(sample_open("COM3")).await.unwrap();
    let mut ws = connect_data(&open.data_url).await.unwrap();
    wait_state(&app, SessionState::Active).await;

    // 同步灌入：主线程不 yield，数据循环无法排空 rx 通道(容量 1)；
    // 读线程读到第二块时 try_send 遇满，触发背压 4005。
    for _ in 0..10 {
        app.serial.push_rx(&[0u8; 4096]);
    }
    // 阻塞主线程一小段，让读线程在没有数据循环竞争的情况下完成两次读取。
    std::thread::sleep(Duration::from_millis(50));

    assert_eq!(close_code(&mut ws).await, 4005);
}
