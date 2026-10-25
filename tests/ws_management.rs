//! 管理 WebSocket：hello、7 个 action 分发、Origin 校验、复用规则，
//! 以及「管理连接断开不影响数据 WS」。

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{sample_port, FakeAdminController, FakeScanner, FakeTransport, FakeTransportFactory};
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
use websocket2serial::ws::heartbeat::Heartbeat;
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
    scanner: Arc<FakeScanner>,
    factory: Arc<FakeTransportFactory>,
    /// 直接 `manager.attach` 拿到的收发通道，保持 rx 存活、会话维持 active。
    attached: Mutex<Vec<AttachedData>>,
}

impl App {
    fn management_port(&self) -> u16 {
        self.built.management_port
    }

    /// 手动触发一次热插拔扫描（对应生产每 2s 的轮询）。
    async fn hotplug_tick(&self) {
        self.built.state.hotplug.tick();
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
    let built = build_app(config, manager, admin, scanner.clone(), events).unwrap();
    App {
        built,
        serial,
        sessions,
        scanner,
        factory: transport_factory,
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

/// 读取下一条指定类型的事件（跳过其他事件，如 `hello`）。
async fn next_event_of(ws: &mut Ws, name: &str) -> Value {
    loop {
        let v = next_event(ws).await;
        if v.get("event").and_then(Value::as_str) == Some(name) {
            return v;
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

/// 短轮询等待会话调试环形缓冲至少 `n` 帧（保证回放有内容可回放）。
async fn wait_debug_frames(app: &App, id: &str, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let len = app
            .built
            .state
            .manager
            .debug_snapshot(id)
            .map(|f| f.len())
            .unwrap_or(0);
        if len >= n {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待调试帧 {n} 条超时（当前 {len}）"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
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

#[tokio::test]
async fn session_state_events_follow_lifecycle() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap().to_string();
    let ev = next_event_of(&mut ws, "session.state").await;
    assert_eq!(ev["data"]["state"], "awaiting");
    assert_eq!(ev["data"]["sessionId"], id);
    app.attach(&id).await;
    let ev = next_event_of(&mut ws, "session.state").await;
    assert_eq!(ev["data"]["state"], "active");
}

#[tokio::test]
async fn debug_subscribe_replays_then_streams_with_seq() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap().to_string();
    app.attach(&id).await;
    // 灌入一条数据并等待其进入调试环形缓冲（seq=1），保证回放有内容。
    app.serial.push_rx(b"before");
    wait_debug_frames(&app, &id, 1).await;
    request(
        &mut ws,
        "debug.subscribe",
        json!({"sessionIds":[id],"replay":true}),
    )
    .await;
    let replayed = next_event_of(&mut ws, "debug.frame").await;
    assert_eq!(replayed["data"]["replay"], true);
    // 再灌入实时数据，seq 应大于回放帧。
    app.serial.push_rx(b"live");
    let live = next_event_of(&mut ws, "debug.frame").await;
    assert!(live["data"]["seq"].as_u64() > replayed["data"]["seq"].as_u64());
}

#[tokio::test]
async fn ports_changed_is_emitted() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    app.scanner.add(sample_port("COM9"));
    app.hotplug_tick().await;
    let ev = next_event_of(&mut ws, "ports.changed").await;
    assert_eq!(ev["data"]["added"][0]["path"], "COM9");
}

#[test]
fn heartbeat_dead_after_ten_seconds_without_pong() {
    let mut h = Heartbeat::new(Duration::from_secs(5), Duration::from_secs(10));
    h.on_pong(Instant::now());
    assert!(!h.is_dead(Instant::now() + Duration::from_secs(9)));
    assert!(h.is_dead(Instant::now() + Duration::from_secs(11)));
}

#[tokio::test]
async fn slow_consumer_is_closed_with_1013_and_reconnect_lists_cleanly() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let _hello = next_event(&mut ws).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap().to_string();
    app.attach(&id).await;
    request(
        &mut ws,
        "debug.subscribe",
        json!({"sessionIds":[id],"replay":false}),
    )
    .await;

    // 客户端不读管理 WS：灌入大量 TX 调试帧填满写队列（debug 帧满则丢，不触发 1013）。
    // 每写之间节流 1ms，避免广播被写线程压垮产生 Lagged（Lagged 也会 1013，会失去区分力）。
    let payload = vec![0u8; 64 * 1024];
    for _ in 0..64 {
        app.built
            .state
            .manager
            .write(&id, payload.clone())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    // 等待写线程写完（tx_bytes 达到预期），保证调试帧已全部生成、写队列已满。
    let expected = (64 * 64 * 1024) as u64;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let tx_bytes = app
            .built
            .state
            .manager
            .list()
            .await
            .iter()
            .find(|s| s.session_id == id)
            .map(|s| s.counters.tx_bytes)
            .unwrap_or(0);
        if tx_bytes >= expected {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "TX 未写完：{tx_bytes}/{expected}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 触发状态事件（ports.changed）→ 写队列满 → 1013 关闭管理连接（不靠 debug 满）。
    app.scanner.add(sample_port("COM9"));
    app.hotplug_tick().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), close_code(&mut ws))
            .await
            .expect("状态事件满未在 5s 内触发 1013"),
        1013
    );

    // 重连后 session.list 仍可查询（会话 active，状态对齐）。
    let mut ws2 = connect_management(&app).await;
    let _hello = next_event(&mut ws2).await;
    let list = request(&mut ws2, "session.list", json!({})).await;
    assert_eq!(list["data"]["sessions"][0]["state"], "active");
}

#[tokio::test]
async fn requests_are_served_during_debug_flood() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let _hello = next_event(&mut ws).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap().to_string();
    app.attach(&id).await;
    request(
        &mut ws,
        "debug.subscribe",
        json!({"sessionIds":"all","replay":false}),
    )
    .await;

    // 拆成 sink/stream：后台读任务持续 drain 事件，避免写队列满触发 1013。
    let (mut sink, mut stream) = ws.split();
    let list_id = "flood-list";
    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    let drain = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            if let Message::Text(text) = msg {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if v.get("id").and_then(Value::as_str) == Some(list_id) {
                        let _ = resp_tx.send(v);
                        break;
                    }
                }
            }
        }
    });

    // 后台持续灌 TX 调试帧（洪峰），1ms 节流避免压垮 drain/写队列。
    let manager = Arc::clone(&app.built.state.manager);
    let flood_id = id.clone();
    let flood = tokio::spawn(async move {
        let payload = vec![0u8; 1024];
        loop {
            if manager.write(&flood_id, payload.clone()).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // 洪峰进行中发 session.list，断言 2s 内得到响应（证明请求没被事件洪峰饿死）。
    sink.send(Message::Text(
        json!({"id": list_id, "action": "session.list", "params": {}}).to_string(),
    ))
    .await
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), resp_rx)
        .await
        .expect("session.list 在 2s 内无响应（被事件洪峰饿死）")
        .unwrap();
    assert!(response["data"]["sessions"].is_array());

    flood.abort();
    drain.abort();
}

#[tokio::test]
async fn broadcast_lag_closes_1013() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let _hello = next_event(&mut ws).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap().to_string();
    app.attach(&id).await;
    request(
        &mut ws,
        "debug.subscribe",
        json!({"sessionIds":"all","replay":false}),
    )
    .await;

    // 用闸门阻塞下一次 open：主循环将卡在 spawn_blocking，无法消费广播。
    let mut gate = app.factory.gate_next_open();
    ws.send(Message::Text(
        json!({
            "id":"lag-open","action":"session.open",
            "params":{"path":"COM4","baudRate":115200,"dataBits":8,
                      "parity":"none","stopBits":1,"flowControl":"none"}
        })
        .to_string(),
    ))
    .await
    .unwrap();
    gate.wait_started().await;

    // 主循环被阻塞期间灌入 >16 条调试帧，使广播容量（16）溢出 → Lagged。
    for _ in 0..40 {
        app.serial.push_rx(b"x");
    }
    // 等读线程把 40 帧都读出（>16 已溢出）后再释放闸门。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rx_bytes = app
            .built
            .state
            .manager
            .list()
            .await
            .iter()
            .find(|s| s.session_id == id)
            .map(|s| s.counters.rx_bytes)
            .unwrap_or(0);
        if rx_bytes >= 40 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "rx 未读完：{rx_bytes}/40"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    gate.release();

    // 主循环随后 recv 到 Lagged → 1013 关闭。
    assert_eq!(close_code(&mut ws).await, 1013);
}

#[tokio::test]
async fn debug_flood_does_not_close_management() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let _hello = next_event(&mut ws).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap().to_string();
    app.attach(&id).await;
    request(
        &mut ws,
        "debug.subscribe",
        json!({"sessionIds":[id],"replay":false}),
    )
    .await;

    // 拆成 sink/stream：后台读任务持续 drain，避免写队列满。
    let (mut sink, mut stream) = ws.split();
    let list_id = "debug-flood-alive";
    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    let drain = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            if let Message::Text(text) = msg {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if v.get("id").and_then(Value::as_str) == Some(list_id) {
                        let _ = resp_tx.send(v);
                        break;
                    }
                }
            }
        }
    });

    // 后台持续灌 TX 调试帧（洪峰，1ms 节流避免广播 Lagged）。
    let manager = Arc::clone(&app.built.state.manager);
    let flood_id = id.clone();
    let flood = tokio::spawn(async move {
        let payload = vec![0u8; 64 * 1024];
        loop {
            if manager.write(&flood_id, payload.clone()).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // 洪峰进行中发 session.list，断言 2s 内响应（debug 洪峰未关闭连接）。
    sink.send(Message::Text(
        json!({"id": list_id, "action": "session.list", "params": {}}).to_string(),
    ))
    .await
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), resp_rx)
        .await
        .expect("debug 洪峰下 session.list 无响应（连接被 1013 关闭）")
        .unwrap();
    assert!(response["data"]["sessions"].is_array());

    flood.abort();
    drain.abort();
}
