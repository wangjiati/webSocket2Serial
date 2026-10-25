use websocket2serial::protocol::{
    ActionName, AdminState, CloseCode, ErrorCode, Event, HelloData, OpenParams, Request,
    SessionSelector,
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
