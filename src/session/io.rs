//! 会话串口 I/O 线程：读线程（串口→网页）与写线程（网页→串口）。
//!
//! 两个阻塞线程共享 `Arc<Mutex<Box<dyn SerialTransport>>>`，首次 `attach` 进入
//! `active` 时由 [`Session::start_io`] 建立；关闭时由 [`IoHandle::shutdown`]
//! 带超时 join 并释放串口。读/写线程自身检测到错误时只调用 `Session::close`
//! 改状态并通知，join 交给 manager 的 `closed_rx` 清理，避免自 join 死锁。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::protocol::SessionState;
use crate::serial::{SerialTransport, TransportError};

use super::session::{CloseReason, Session};

/// 单次读缓冲字节数（与 rx 队列容量单位 4096 对齐）。
const READ_BUF: usize = 4096;

/// 写线程分块的最小/最大字节数。
const CHUNK_MIN: u32 = 256;
const CHUNK_MAX: u32 = 4096;

/// join 线程的超时；超时后放弃等待（detach），避免关闭被卡住。
const JOIN_TIMEOUT: Duration = Duration::from_secs(1);

/// 运行中的读/写线程句柄与共享串口传输。
pub(crate) struct IoHandle {
    transport: Arc<Mutex<Box<dyn SerialTransport>>>,
    read: std::thread::JoinHandle<()>,
    write: std::thread::JoinHandle<()>,
}

impl IoHandle {
    /// 建立读/写线程并返回句柄。
    pub(crate) fn spawn(
        session: Arc<Session>,
        transport: Box<dyn SerialTransport>,
        rx_tx: mpsc::Sender<Vec<u8>>,
        tx_rx: mpsc::Receiver<Vec<u8>>,
    ) -> Self {
        let transport = Arc::new(Mutex::new(transport));
        let read = {
            let session = Arc::clone(&session);
            let transport = Arc::clone(&transport);
            std::thread::spawn(move || read_loop(session, transport, rx_tx))
        };
        let write = {
            let session = Arc::clone(&session);
            let transport = Arc::clone(&transport);
            std::thread::spawn(move || write_loop(session, transport, tx_rx))
        };
        Self {
            transport,
            read,
            write,
        }
    }

    /// 先请线程退出（状态已由 `Session::close` 置 Closed），再带超时 join 并释放串口。
    pub(crate) fn shutdown(self) {
        join_timeout(self.read);
        join_timeout(self.write);
        // 显式 drop 串口传输，释放串口句柄（RAII 在此收尾）。
        drop(self.transport);
    }
}

/// 读线程：5ms 超时循环，`active` 交付、`grace` 丢弃，`DeviceLost`→4001、其他 I/O→4007。
fn read_loop(
    session: Arc<Session>,
    transport: Arc<Mutex<Box<dyn SerialTransport>>>,
    rx_tx: mpsc::Sender<Vec<u8>>,
) {
    let read_timeout = Duration::from_millis(session.read_timeout_ms());
    let mut buf = [0u8; READ_BUF];
    loop {
        if session.state() == SessionState::Closed {
            break;
        }
        let started = Instant::now();
        let read = {
            let mut transport = transport.lock().unwrap();
            transport.read(&mut buf, read_timeout)
        };
        match read {
            Ok(0) => continue,
            Ok(n) => {
                // M2 统计：read 返回 → try_send 前的耗时。
                tracing::debug!(
                    elapsed_us = started.elapsed().as_micros() as u64,
                    "rx read→try_send"
                );
                match session.state() {
                    SessionState::Active => match rx_tx.try_send(buf[..n].to_vec()) {
                        Ok(()) => {
                            session.record_rx(n as u64);
                            session.push_debug("rx", buf[..n].to_vec());
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            session.close(CloseReason::Backpressure);
                            break;
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    },
                    SessionState::Grace => {
                        session.record_discarded(n as u64);
                    }
                    // awaiting 不会出现（线程仅在 active 启动）；closed 已在上方退出。
                    _ => break,
                }
            }
            Err(TransportError::Timeout) => continue,
            Err(TransportError::DeviceLost) => {
                session.close(CloseReason::DeviceLost);
                break;
            }
            Err(_) => {
                session.close(CloseReason::SerialIo);
                break;
            }
        }
    }
}

/// 写线程：从容量 1 的 tokio mpsc 取 `Vec<u8>`，按 `clamp(baud/100, 256, 4096)` 分块写。
fn write_loop(
    session: Arc<Session>,
    transport: Arc<Mutex<Box<dyn SerialTransport>>>,
    mut tx_rx: mpsc::Receiver<Vec<u8>>,
) {
    let write_timeout = Duration::from_millis(session.write_timeout_ms());
    let chunk = (session.baud_rate() / 100).clamp(CHUNK_MIN, CHUNK_MAX) as usize;
    // `blocking_recv` 返回 None 表示所有发送端已 drop（channel 关闭），写线程退出。
    while let Some(data) = tx_rx.blocking_recv() {
        if session.state() == SessionState::Closed {
            break;
        }
        let started = Instant::now();
        for part in data.chunks(chunk) {
            let written = {
                let mut transport = transport.lock().unwrap();
                transport.write(part, write_timeout)
            };
            match written {
                Ok(n) => {
                    session.record_tx(n as u64);
                    session.push_debug("tx", part.to_vec());
                }
                Err(TransportError::WriteTimeout) => {
                    session.close(CloseReason::WriteTimeout);
                    return;
                }
                Err(TransportError::DeviceLost) => {
                    session.close(CloseReason::DeviceLost);
                    return;
                }
                Err(_) => {
                    session.close(CloseReason::SerialIo);
                    return;
                }
            }
        }
        // M4 统计：recv → write 返回后的耗时。
        tracing::debug!(
            elapsed_us = started.elapsed().as_micros() as u64,
            "tx recv→write返回"
        );
    }
}

/// 带超时 join；超时则放弃等待（drop handle，线程 detach）。
fn join_timeout(handle: std::thread::JoinHandle<()>) {
    let deadline = Instant::now() + JOIN_TIMEOUT;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = handle.join();
}
