//! 管理 WebSocket：hello、7 个 action 分发、Origin 校验、复用规则，
//! 以及「管理连接断开不影响数据 WS」。

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeAdminController, FakeScanner, FakeTransport, FakeTransportFactory};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use websocket2serial::app::build_app;
use websocket2serial::config::{Config, OriginAllowlist, ServerConfig, SessionConfig};
use websocket2serial::protocol::SessionState;
use websocket2serial::session::{
    AttachedData, DataError, DataListenerFactory, DataListenerHandle, Session, SessionManager,
};
use websocket2serial::ws::AxumDataListenerFactory;

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// 数据/管理 WS 客户端统一携带的允许 Origin。
const ALLOWED_ORIGIN: &str = "http://127.0.0.1:1234";

/// 记录每次 `bind` 的 `Session`，供测试侧 `attach` 取 token、连数据 WS。
struct RecordingDataListener {
    inner: AxumDataListenerFactory,
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
}

impl DataListenerFactory for RecordingDataListener {
    fn bind(&self, session: Arc<Session>) -> Result<DataListenerHandle, DataError> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id().to_string(), Arc::clone(&session));
        self.inner.bind(session)
    }
}

/// 测试应用：真实数据监听器 + 假串口 + 假扫描器/管理控制器 + 管理 WS。
struct App {
    built: websocket2serial::app::App,
    serial: FakeTransport,
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    /// 直接 `manager.attach` 拿到的收发通道，保持 rx 存活、会话维持 active。
    attached: Mutex<Vec<AttachedData>>,
}

impl App {
    fn management_port(&self) -> u16 {
        self.built.management_port
    }

    /// 直接 `manager.attach`：同步转入 active 并启动读写线程，避免数据 WS 握手竞态。
    async fn attach(&self, id: &str) {
        let token = self
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .expect("session 未记录")
            .current_token();
        let attached = self
            .built
            .state
            .manager
            .attach(id, &token)
            .await
            .expect("attach 失败");
        self.attached.lock().unwrap().push(attached);
    }

    /// 连接真实数据 WS（会话须已 open）。
    async fn connect_data(&self, id: &str) -> Ws {
        let url = {
            let sessions = self.sessions.lock().unwrap();
            let session = sessions.get(id).expect("session 未记录");
            format!(
                "ws://127.0.0.1:{}/ws/{}",
                session.data_port(),
                session.current_token()
            )
        };
        connect_data_ws(&url).await.unwrap()
    }
}

async fn test_app() -> App {
    let serial = FakeTransport::new();
    let transport_factory = Arc::new(FakeTransportFactory::with_transport(serial.clone()));
    let origins = OriginAllowlist::new(&[
        "http://127.0.0.1:*".to_string(),
        "http://localhost:*".to_string(),
    ]);
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let (events, _) = broadcast::channel(16);
    let cfg = SessionConfig::default();
    let manager = Arc::new_cyclic(|weak| {
        let inner = AxumDataListenerFactory::new(weak.clone(), origins.clone(), cfg.tx_queue_bytes);
        let factory = Arc::new(RecordingDataListener {
            inner,
            sessions: Arc::clone(&sessions),
        });
        SessionManager::new(
            transport_factory.clone(),
            factory,
            cfg.clone(),
            events.clone(),
        )
    });
    let config = Config {
        server: ServerConfig { management_port: 0 },
        ..Config::default()
    };
    let admin = Arc::new(FakeAdminController::new());
    let scanner = Arc::new(FakeScanner::new());
    let built = build_app(config, manager, admin, scanner, events).unwrap();
    App {
        built,
        serial,
        sessions,
        attached: Mutex::new(Vec::new()),
    }
}

/// 连接数据 WS，携带允许的 Origin。
async fn connect_data_ws(url: &str) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("origin", http::HeaderValue::from_static(ALLOWED_ORIGIN));
    let (ws, _response) = connect_async(request).await?;
    Ok(ws)
}

/// 连接管理 WS，携带允许的 Origin。
async fn connect_management(app: &App) -> Ws {
    connect_management_with_origin(app, ALLOWED_ORIGIN)
        .await
        .unwrap()
}

/// 以指定 Origin 连接管理 WS；Origin 不合法时返回 Err。
async fn connect_management_with_origin(
    app: &App,
    origin: &str,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let url = format!("ws://127.0.0.1:{}/ws", app.management_port());
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "origin",
        http::HeaderValue::from_str(origin).expect("非法 Origin 头"),
    );
    let (ws, _response) = connect_async(request).await?;
    Ok(ws)
}

/// 读取下一条文本帧并解析为 JSON。
async fn next_json(ws: &mut Ws) -> Value {
    loop {
        match ws.next().await.expect("流结束").expect("读取错误") {
            Message::Text(text) => return serde_json::from_str(&text).expect("非法 JSON"),
            _ => continue,
        }
    }
}

/// 读取下一条事件帧（带 `event` 字段）。
async fn next_event(ws: &mut Ws) -> Value {
    let v = next_json(ws).await;
    assert!(v.get("event").is_some(), "期望事件，收到 {v}");
    v
}

static REQ_ID: AtomicU64 = AtomicU64::new(0);

/// 发送请求并读取匹配 `id` 的应答（跳过穿插的事件帧）。
async fn request(ws: &mut Ws, action: &str, params: Value) -> Value {
    let id = format!("r{}", REQ_ID.fetch_add(1, Ordering::Relaxed));
    let msg = json!({"id": id, "action": action, "params": params});
    ws.send(Message::Text(msg.to_string())).await.unwrap();
    loop {
        let v = next_json(ws).await;
        if v.get("id").and_then(Value::as_str) == Some(id.as_str()) {
            return v;
        }
    }
}

fn open_params() -> Value {
    json!({
        "path":"COM3","baudRate":115200,"dataBits":8,
        "parity":"none","stopBits":1,"flowControl":"none"
    })
}

/// 读取下一条二进制帧（跳过控制帧）。
async fn next_binary(ws: &mut Ws) -> Vec<u8> {
    loop {
        match ws.next().await.expect("流结束").expect("读取错误") {
            Message::Binary(data) => return data,
            _ => continue,
        }
    }
}

/// 短轮询等待会话进入指定状态。
async fn wait_state(app: &App, id: &str, expected: SessionState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state = app
            .built
            .state
            .manager
            .list()
            .await
            .iter()
            .find(|s| s.session_id == id)
            .map(|s| s.state);
        if state == Some(expected) {
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
async fn hello_then_ports_list_and_open_list_close() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let hello = next_event(&mut ws).await;
    assert_eq!(hello["event"], "hello");
    let ports = request(&mut ws, "ports.list", json!({})).await;
    assert!(ports["data"]["ports"].is_array());
    let open = request(
        &mut ws,
        "session.open",
        json!({
            "path":"COM3","baudRate":115200,"dataBits":8,
            "parity":"none","stopBits":1,"flowControl":"none"
        }),
    )
    .await;
    assert!(open["data"]["dataUrl"]
        .as_str()
        .unwrap()
        .starts_with("ws://127.0.0.1:"));
    let list = request(&mut ws, "session.list", json!({})).await;
    assert_eq!(list["data"]["sessions"][0]["state"], "awaiting");
    let id = open["data"]["sessionId"].as_str().unwrap();
    let closed = request(&mut ws, "session.close", json!({"sessionId": id})).await;
    assert_eq!(closed["data"]["closed"], true);
}

#[tokio::test]
async fn reuse_rules_through_management_ws() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let a = request(&mut ws, "session.open", open_params()).await;
    let b = request(&mut ws, "session.open", open_params()).await;
    assert_eq!(b["data"]["reused"], true);
    assert_ne!(a["data"]["dataUrl"], b["data"]["dataUrl"]);
    app.attach(a["data"]["sessionId"].as_str().unwrap()).await;
    let c = request(&mut ws, "session.open", open_params()).await;
    assert_eq!(c["error"]["code"], "PORT_BUSY");
}

#[tokio::test]
async fn bad_origin_is_rejected() {
    let app = test_app().await;
    let err = connect_management_with_origin(&app, "http://evil.example").await;
    assert!(err.is_err());
}

#[tokio::test]
async fn invalid_action_returns_error() {
    let mut ws = connect_management(&test_app().await).await;
    let r = request(&mut ws, "nope", json!({})).await;
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"]["code"], "UNKNOWN_ACTION");
}

#[tokio::test]
async fn admin_set_and_session_write_are_dispatched() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let r = request(&mut ws, "admin.set", json!({"enabled":false})).await;
    assert_eq!(r["data"]["enabled"], false);
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap();
    let err = request(
        &mut ws,
        "session.write",
        json!({"sessionId":id,"dataB64":"aGk="}),
    )
    .await;
    assert_eq!(err["error"]["code"], "SESSION_STATE");
    app.attach(id).await;
    let ok = request(
        &mut ws,
        "session.write",
        json!({"sessionId":id,"dataB64":"aGk="}),
    )
    .await;
    assert_eq!(ok["data"]["written"], 2);
    app.serial.expect_tx(b"hi").await;
}

#[tokio::test]
async fn management_disconnect_does_not_affect_data_ws() {
    let app = test_app().await;
    let mut mgmt = connect_management(&app).await;
    let _hello = next_event(&mut mgmt).await;
    let open = request(&mut mgmt, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap();

    // 建立数据 WS 并等待进入 active。
    let mut data = app.connect_data(id).await;
    wait_state(&app, id, SessionState::Active).await;

    // 关闭管理连接，数据 WS 仍能双向收发。
    drop(mgmt);
    data.send(Message::Binary(b"ping".to_vec())).await.unwrap();
    app.serial.expect_tx(b"ping").await;
    app.serial.push_rx(b"pong");
    assert_eq!(next_binary(&mut data).await, b"pong");

    // 重连管理，session.list 状态一致（active）。
    let mut mgmt2 = connect_management(&app).await;
    let _hello = next_event(&mut mgmt2).await;
    let list = request(&mut mgmt2, "session.list", json!({})).await;
    assert_eq!(list["data"]["sessions"][0]["state"], "active");
}
