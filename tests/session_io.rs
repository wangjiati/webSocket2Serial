//! 会话 I/O：banner 保留、写停滞 4007、背压 4005、掉线 4001 与宽限丢弃/恢复。

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    sample_open, sample_params, test_manager_with_drop_notifying_serial,
    test_manager_with_fake_serial, test_manager_with_small_rx_queue, token_of, wait_closed,
    FakeTransport,
};
use tokio::sync::{broadcast, mpsc};
use websocket2serial::config::SessionConfig;
use websocket2serial::serial::TransportError;
use websocket2serial::session::{CloseReason, Session};

use std::sync::atomic::Ordering;

#[tokio::test]
async fn attach_delivers_pre_attach_banner() {
    let (mgr, fake) = test_manager_with_fake_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    fake.push_rx(b"READY\n");
    tokio::time::sleep(Duration::from_millis(50)).await; // awaiting 不读
    let mut attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(1), attached.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"READY\n");
}

#[tokio::test]
async fn write_stall_closes_4007_and_releases_port() {
    let (mgr, fake) = test_manager_with_fake_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    fake.fail_next_write(TransportError::WriteTimeout);
    attached.tx.send(b"x".to_vec()).await.unwrap();
    let reason = wait_closed(&mgr, &r.session_id).await;
    assert_eq!(reason.code, 4007);
    assert!(mgr.list().await.is_empty());
}

#[tokio::test]
async fn rx_backpressure_closes_4005() {
    let (mgr, fake) = test_manager_with_small_rx_queue();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let _attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    for _ in 0..10 {
        fake.push_rx(&[0u8; 4096]);
    }
    let reason = wait_closed(&mgr, &r.session_id).await;
    assert_eq!(reason.code, 4005);
}

#[tokio::test]
async fn device_loss_closes_4001() {
    let (mgr, fake) = test_manager_with_fake_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let _attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    fake.fail_next_read(TransportError::DeviceLost);
    let reason = wait_closed(&mgr, &r.session_id).await;
    assert_eq!(reason.code, 4001);
}

#[tokio::test]
async fn grace_discards_and_resumes() {
    let fake = FakeTransport::new();
    let (events, _) = broadcast::channel(16);
    let (closed_tx, _closed_rx) = mpsc::unbounded_channel();
    let session = Arc::new(Session::with_transport(
        "s1",
        sample_params("COM3"),
        SessionConfig::default(),
        events,
        closed_tx,
        Some(Box::new(fake.clone())),
    ));

    // active：数据正常交付。
    let mut attached = session.attach().unwrap();
    fake.push_rx(b"ONE");
    let got = tokio::time::timeout(Duration::from_secs(1), attached.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"ONE");

    // 模拟数据 WS 断开：归还接收端并进入宽限；宽限内读到但不交付。
    session.return_rx(attached.rx);
    session.enter_grace().unwrap();
    fake.push_rx(b"DROP");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while session.counters().rx_discarded_bytes < 4 {
        assert!(tokio::time::Instant::now() < deadline, "宽限丢弃计数未增长");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 重连：恢复 active，继续交付后续数据。
    let mut re = session.attach().unwrap();
    fake.push_rx(b"TWO");
    let got = tokio::time::timeout(Duration::from_secs(1), re.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"TWO");
}

#[tokio::test]
async fn writer_exits_on_shutdown_even_with_tx_clone() {
    let (mgr, _fake, dropped) = test_manager_with_drop_notifying_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    // 保留一个 tx 克隆，模拟数据 WS 仍持有发送端、未 drop。
    let retained_tx = attached.tx.clone();
    drop(attached);

    mgr.close(&r.session_id, CloseReason::AdminClosed)
        .await
        .unwrap();

    // 写线程应通过独立 shutdown 信号退出，串口对象在 1s 内释放，不依赖 tx 克隆 drop。
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !dropped.load(Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "串口未在 1s 内释放（写线程仍持有传输）"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(retained_tx);
}

#[tokio::test]
async fn close_does_not_block_runtime() {
    let (mgr, _fake) = test_manager_with_fake_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let _attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();

    let started = std::time::Instant::now();
    mgr.close(&r.session_id, CloseReason::ClientClosed)
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(200),
        "close 阻塞运行时：{elapsed:?}"
    );
}
