//! 连接心跳状态机：记录最近 Pong 时刻，判定连接是否已死。
//!
//! 连接层据此周期性发 Ping，并据此判断是否超时；本模块只负责时间判定，
//! 不直接收发帧，便于单元测试注入任意时刻。

use std::time::{Duration, Instant};

/// 心跳状态机。
///
/// - `on_pong` 在连接建立与每次收到 Pong 时调用，刷新最近 Pong 时刻；
/// - `is_dead` 判定距最近 Pong 是否已超过 `dead_after`。
#[derive(Debug, Clone)]
pub struct Heartbeat {
    ping_interval: Duration,
    dead_after: Duration,
    last_pong: Option<Instant>,
}

impl Heartbeat {
    /// 以 `ping_interval`（发 Ping 间隔）与 `dead_after`（判死阈值）构造。
    pub fn new(ping_interval: Duration, dead_after: Duration) -> Self {
        Self {
            ping_interval,
            dead_after,
            last_pong: None,
        }
    }

    /// 记录收到 Pong（或连接建立）的时刻。
    pub fn on_pong(&mut self, now: Instant) {
        self.last_pong = Some(now);
    }

    /// 距最近 Pong 是否已超过 `dead_after`；从未收到 Pong 视为已死。
    pub fn is_dead(&self, now: Instant) -> bool {
        match self.last_pong {
            Some(last) => now.duration_since(last) > self.dead_after,
            None => true,
        }
    }

    /// 发 Ping 的间隔。
    pub fn ping_interval(&self) -> Duration {
        self.ping_interval
    }
}
