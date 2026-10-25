use clap::Parser;
use websocket2serial::{Cli, Config, OriginAllowlist};

#[test]
fn defaults_match_spec() {
    let c = Config::default();
    assert_eq!(c.server.management_port, 8765);
    assert_eq!(c.admin.port, 8766);
    assert!(c.admin.enabled);
    assert_eq!(c.session.connect_timeout_ms, 5000);
    assert_eq!(c.session.grace_ms, 10000);
    assert_eq!(c.session.max_sessions, 8);
    assert_eq!(c.session.read_timeout_ms, 5);
    assert_eq!(c.session.write_timeout_ms, 5000);
    assert_eq!(c.session.rx_queue_bytes, 1_048_576);
    assert_eq!(c.session.tx_queue_bytes, 1_048_576);
}

#[test]
fn unknown_config_field_falls_back_to_defaults_with_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[session]\nconnect_timeout_ms = 1\ntyppo = 2\n").unwrap();
    let (cfg, err) = Config::load_or_default(&path);
    assert!(err.is_some());
    assert_eq!(cfg.session.connect_timeout_ms, 5000);
}

#[test]
fn origin_allowlist_allows_loopback_and_rejects_remote() {
    let a = OriginAllowlist::new(&["http://127.0.0.1:*".into(), "http://localhost:*".into()]);
    assert!(a.allows(Some("http://127.0.0.1:3000")));
    assert!(a.allows(Some("http://localhost:8080")));
    assert!(!a.allows(Some("http://evil.example")));
    assert!(!a.allows(None));
}

#[test]
fn cli_admin_flag_is_parsed() {
    let cli = Cli::parse_from(["ws2serial", "--admin"]);
    assert!(cli.force_admin);
}
