//! 关闭阶段的有界线程等待，避免模态 UI 永久阻塞核心服务清理。

use std::time::{Duration, Instant};

/// 在 `timeout` 内等待线程结束；超时后放弃等待并分离线程。
pub fn join_bounded(handle: std::thread::JoinHandle<()>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    handle.join().is_ok()
}
