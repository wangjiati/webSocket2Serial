use websocket2serial::protocol::{
    ActionName, AdminState, CloseCode, Counters, DebugFrame, DebugFrameData, ErrorBody, ErrorCode,
    Event, FlowControl, HelloData, OpenParams, Parity, PortsChangedData, Request, SessionParams,
    SessionSelector, SessionState, SessionStateData,
};

#[test]
fn parses_open_request_with_camel_case_params() {
    let raw = r#"{"id":"1","action":"session.open","params":{
        "path":"COM3","baudRate":115200,"dataBits":8,"parity":"none",
        "stopBits":1,"flowControl":"none","dtr":true,"rts":false}}"#;
    let req: Request = serde_json::from_str(raw).unwrap();
    assert_eq!(req.action, ActionName::SessionOpen);
    let p: OpenParams = serde_json::from_value(req.params).unwrap();
    assert_eq!(p.baud_rate, 115200);
    assert_eq!(p.path, "COM3");
    assert_eq!(p.parity, Parity::None);
    assert_eq!(p.flow_control, FlowControl::None);
}

#[test]
fn serializes_hello_event_with_event_name() {
    let e = Event::Hello(HelloData {
        protocol: "ws2s/v1".into(),
        version: "0.1.0".into(),
        os: "windows".into(),
        arch: "x86_64".into(),
        management_port: 8765,
        admin: AdminState {
            enabled: true,
            url: "http://127.0.0.1:8766/".into(),
        },
        autostart: true,
    });
    let v = serde_json::to_value(e).unwrap();
    assert_eq!(v["event"], "hello");
    assert_eq!(v["data"]["managementPort"], 8765);
}

#[test]
fn session_selector_accepts_all_or_ids() {
    let all: SessionSelector = serde_json::from_str("\"all\"").unwrap();
    let ids: SessionSelector = serde_json::from_str(r#"["a","b"]"#).unwrap();
    assert!(matches!(all, SessionSelector::All));
    assert!(matches!(ids, SessionSelector::Ids(v) if v.len() == 2));
}

#[test]
fn error_and_close_codes_serialize_as_specified() {
    assert_eq!(
        serde_json::to_string(&ErrorCode::PortBusy).unwrap(),
        r#""PORT_BUSY""#
    );
    assert_eq!(CloseCode::DeviceLost.as_u16(), 4001);
}

fn sample_session_state() -> SessionStateData {
    SessionStateData {
        session_id: "s1".into(),
        state: SessionState::Grace,
        port: "COM3".into(),
        params: SessionParams {
            baud_rate: 115200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            flow_control: FlowControl::None,
        },
        reason: None,
        close_code: None,
        grace_ends_at: Some(1760000010000),
        counters: Counters::default(),
        last_gap: None,
        last_error: None,
        last_rx_at: None,
        last_tx_at: None,
    }
}

#[test]
fn all_event_names_use_spec_dots() {
    let events = [
        Event::Hello(HelloData {
            protocol: "ws2s/v1".into(),
            version: "0.1.0".into(),
            os: "windows".into(),
            arch: "x86_64".into(),
            management_port: 8765,
            admin: AdminState {
                enabled: true,
                url: "http://127.0.0.1:8766/".into(),
            },
            autostart: true,
        }),
        Event::PortsChanged(PortsChangedData {
            added: vec![],
            removed: vec![],
        }),
        Event::SessionState(sample_session_state()),
        Event::DebugFrame(DebugFrameData {
            session_id: "s1".into(),
            frame: DebugFrame::new("rx", 1, 0, vec![0u8; 2]),
        }),
        Event::AdminState(AdminState {
            enabled: true,
            url: "http://127.0.0.1:8766/".into(),
        }),
    ];
    let names: Vec<serde_json::Value> = events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap()["event"].clone())
        .collect();
    assert_eq!(
        names,
        vec![
            serde_json::json!("hello"),
            serde_json::json!("ports.changed"),
            serde_json::json!("session.state"),
            serde_json::json!("debug.frame"),
            serde_json::json!("admin.state"),
        ]
    );
}

#[test]
fn session_state_event_keeps_null_last_gap_and_camel_case_fields() {
    let v = serde_json::to_value(Event::SessionState(sample_session_state())).unwrap();
    assert_eq!(v["event"], "session.state");
    let data = &v["data"];
    assert_eq!(data["sessionId"], "s1");
    assert_eq!(data["state"], "grace");
    assert_eq!(data["port"], "COM3");
    assert_eq!(data["params"]["baudRate"], 115200);
    assert_eq!(data["params"]["parity"], "none");
    assert_eq!(data["params"]["flowControl"], "none");
    assert!(data["counters"].is_object());
    assert!(data.get("lastGap").is_some());
    assert!(data["lastGap"].is_null());
}

#[test]
fn debug_frame_event_has_seq_and_base64_shape() {
    let e = Event::DebugFrame(DebugFrameData {
        session_id: "s1".into(),
        frame: DebugFrame::new("rx", 7, 1760000000000, b"hi".to_vec()),
    });
    let v = serde_json::to_value(e).unwrap();
    assert_eq!(v["event"], "debug.frame");
    assert_eq!(v["data"]["sessionId"], "s1");
    assert_eq!(v["data"]["seq"], 7);
    assert_eq!(v["data"]["tsMs"], 1760000000000u64);
    assert_eq!(v["data"]["size"], 2);
    assert_eq!(v["data"]["dataB64"], "aGk=");
    assert!(v["data"].get("replay").is_none());
}

#[test]
fn unknown_action_maps_to_unknown_and_unknown_action_error() {
    let req: Request = serde_json::from_str(r#"{"id":"1","action":"nope","params":{}}"#).unwrap();
    assert_eq!(req.action, ActionName::Unknown);
    assert_eq!(
        serde_json::to_value(ActionName::SessionOpen).unwrap(),
        "session.open"
    );
    let err = ErrorBody::new(ErrorCode::UnknownAction, "unknown action");
    assert_eq!(serde_json::to_value(err).unwrap()["code"], "UNKNOWN_ACTION");
}

#[test]
fn rejects_unknown_parity_or_flow_control() {
    let base = serde_json::json!({
        "path": "COM3", "baudRate": 115200, "dataBits": 8,
        "parity": "none", "stopBits": 1, "flowControl": "none"
    });

    let mut bad_parity = base.clone();
    bad_parity["parity"] = serde_json::json!("7bit");
    assert!(serde_json::from_value::<OpenParams>(bad_parity).is_err());

    let mut bad_flow = base;
    bad_flow["flowControl"] = serde_json::json!("xonxoff");
    assert!(serde_json::from_value::<OpenParams>(bad_flow).is_err());
}
