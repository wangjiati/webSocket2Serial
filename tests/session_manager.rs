//! 会话管理器：open/close/list、端口独占、复用与定时器。

mod common;

use std::time::Duration;

use common::{sample_open, test_manager, test_manager_with_max_sessions, token_of};
use websocket2serial::protocol::SessionState;
use websocket2serial::session::{CloseReason, ManagerError};

#[tokio::test]
async fn open_returns_awaiting_session_and_binds_data_port() {
    let (mgr, factory) = test_manager();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(!r.reused);
    assert!(r.data_url.starts_with("ws://127.0.0.1:"));
    assert_eq!(mgr.list().await[0].state, SessionState::Awaiting);
    assert_eq!(factory.open_count(), 1);
}

#[tokio::test]
async fn reuse_rotates_token_without_reopening() {
    let (mgr, factory) = test_manager();
    let first = mgr.open(sample_open("COM3")).await.unwrap();
    let second = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(second.reused);
    assert_eq!(second.session_id, first.session_id);
    assert_ne!(second.data_url, first.data_url);
    assert_eq!(factory.open_count(), 1);
    assert!(mgr
        .attach(&first.session_id, &token_of(&first))
        .await
        .is_err());
}

#[tokio::test]
async fn active_open_is_port_busy() {
    let (mgr, _factory) = test_manager();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    let err = mgr.open(sample_open("COM3")).await.unwrap_err();
    assert!(matches!(err, ManagerError::PortBusy));
}

#[tokio::test(start_paused = true)]
async fn awaiting_times_out_after_five_seconds() {
    let (mgr, _factory) = test_manager();
    mgr.open(sample_open("COM3")).await.unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(mgr.list().await.is_empty());
}

#[tokio::test]
async fn close_releases_port_and_can_reopen() {
    let (mgr, _factory) = test_manager();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    mgr.close(&r.session_id, CloseReason::ClientClosed)
        .await
        .unwrap();
    assert!(mgr.list().await.is_empty());
    let again = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(!again.reused);
}

#[tokio::test]
async fn max_sessions_is_enforced() {
    let (mgr, _factory) = test_manager_with_max_sessions(1);
    mgr.open(sample_open("COM3")).await.unwrap();
    let err = mgr.open(sample_open("COM4")).await.unwrap_err();
    assert!(matches!(err, ManagerError::LimitExceeded));
}

#[tokio::test(start_paused = true)]
async fn reuse_resets_awaiting_deadline() {
    let (mgr, _factory) = test_manager();
    mgr.open(sample_open("COM3")).await.unwrap();
    tokio::time::advance(Duration::from_secs(4)).await;
    // 4s 后复用：连接截止被重置为 5s，旧 timer 不应提前杀掉会话。
    mgr.open(sample_open("COM3")).await.unwrap();
    tokio::time::advance(Duration::from_secs(4)).await;
    assert_eq!(mgr.list().await.len(), 1);
    assert_eq!(mgr.list().await[0].state, SessionState::Awaiting);
    // 再过 1s 越过新的 5s 截止，会话才销毁。
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(mgr.list().await.is_empty());
}

#[tokio::test]
async fn concurrent_open_same_path_is_exclusive() {
    let (mgr, factory) = test_manager();
    // 让第一次 open 进入工厂后阻塞，制造并发窗口。
    let mut gate = factory.gate_next_open();

    let mgr = std::sync::Arc::new(mgr);
    let first_mgr = std::sync::Arc::clone(&mgr);
    let first = tokio::spawn(async move { first_mgr.open(sample_open("COM3")).await });

    // 等第一次 open 已占位并进入工厂，再发起第二次同 path open。
    gate.wait_started().await;
    let second = mgr.open(sample_open("COM3")).await;

    gate.release();
    let first = first.await.unwrap();

    // 一次成功、一次 PortBusy；只有一次成功打开；端口锁归成功会话。
    let opened = first.unwrap();
    assert!(matches!(second.unwrap_err(), ManagerError::PortBusy));
    assert_eq!(factory.open_count(), 1);
    let list = mgr.list().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].port, "COM3");
    assert_eq!(list[0].session_id, opened.session_id);
    // 端口锁已归成功会话：再次 open 走复用（而非新建）。
    let again = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(again.reused);
    assert_eq!(again.session_id, opened.session_id);
}

#[tokio::test]
async fn concurrent_max_sessions_counts_reservation() {
    let (mgr, factory) = test_manager_with_max_sessions(1);
    // 让第一次 open 进入工厂后阻塞，此时它已占位并计入上限。
    let mut gate = factory.gate_next_open();

    let mgr = std::sync::Arc::new(mgr);
    let first_mgr = std::sync::Arc::clone(&mgr);
    let first = tokio::spawn(async move { first_mgr.open(sample_open("COM3")).await });

    gate.wait_started().await;
    let second = mgr.open(sample_open("COM4")).await;

    gate.release();
    let first = first.await.unwrap();

    assert!(first.is_ok());
    assert!(matches!(second.unwrap_err(), ManagerError::LimitExceeded));
    assert_eq!(factory.open_count(), 1);
}
