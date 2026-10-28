use std::time::Duration;

use websocket2serial::platform::threads::join_bounded;

#[test]
fn join_bounded_times_out_without_blocking_shutdown() {
    let handle = std::thread::spawn(|| std::thread::sleep(Duration::from_millis(200)));
    assert!(!join_bounded(handle, Duration::from_millis(20)));
}

#[test]
fn join_bounded_joins_a_finished_thread() {
    let handle = std::thread::spawn(|| {});
    assert!(join_bounded(handle, Duration::from_secs(1)));
}
