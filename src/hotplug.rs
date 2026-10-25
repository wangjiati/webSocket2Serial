//! 串口热插拔轮询：按固定间隔扫描，差集非空时广播 `ports.changed`。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;

use crate::protocol::{Event, PortInfo, PortsChangedData, RemovedPort};
use crate::serial::{PortDiff, PortScanner};

/// 默认轮询间隔：2s。
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 热插拔轮询器：持有扫描器、事件广播与上一次端口快照。
///
/// [`run`] 启动后立即扫描一次，此后每 `interval` 扫描一次；每次扫描用
/// [`PortDiff::between`] 与上次快照做差集，非空则广播 `ports.changed` 并更新快照。
/// 扫描失败只记日志，不更新快照也不崩溃。
pub struct Hotplug {
    scanner: Arc<dyn PortScanner>,
    events: broadcast::Sender<Event>,
    previous: Mutex<Vec<PortInfo>>,
}

impl Hotplug {
    pub fn new(scanner: Arc<dyn PortScanner>, events: broadcast::Sender<Event>) -> Self {
        Self {
            scanner,
            events,
            previous: Mutex::new(Vec::new()),
        }
    }

    /// 单次扫描：差集非空时广播 `ports.changed` 并更新快照。
    ///
    /// 返回是否广播了事件（供测试断言）；扫描失败返回 `false`。
    pub fn tick(&self) -> bool {
        match self.scanner.scan() {
            Ok(current) => {
                let mut previous = self.previous.lock().unwrap();
                let diff = PortDiff::between(&previous, &current);
                if diff.is_empty() {
                    *previous = current;
                    return false;
                }
                *previous = current;
                let data = PortsChangedData {
                    added: diff.added,
                    removed: diff
                        .removed
                        .into_iter()
                        .map(|path| RemovedPort { path })
                        .collect(),
                };
                let _ = self.events.send(Event::PortsChanged(data));
                true
            }
            Err(err) => {
                tracing::warn!(error = %err, "串口扫描失败");
                false
            }
        }
    }

    /// 启动轮询循环：立即扫描一次，此后每 `interval` 扫描一次（永不返回）。
    pub async fn run(&self, interval: Duration) {
        let mut timer = tokio::time::interval(interval);
        loop {
            // 首次 tick 立即触发，此后每 `interval` 一次。
            timer.tick().await;
            self.tick();
        }
    }
}
