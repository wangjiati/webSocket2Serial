//! 调试帧环形缓冲：按数据字节上限保存最近帧，超出后丢弃最旧帧。

use std::collections::VecDeque;

use crate::protocol::DebugFrame;

/// 会话调试帧内存缓冲，按原始数据字节数（`DebugFrame.size`）设上限。
///
/// 插入顺序即时间顺序；上限不足时逐帧丢弃最旧的帧（至少保留最新一帧），
/// 保证 `snapshot` 返回的帧仍按 `seq` 单调、连续（若调用方按序编号）。
pub struct DebugRing {
    /// 保留帧的数据字节上限。
    byte_limit: usize,
    /// 当前保留帧的数据字节总数。
    bytes: usize,
    frames: VecDeque<DebugFrame>,
    /// 下一个分配的调试帧序号（从 1 递增）；与 `frames` 同锁分配，保证并发下有序。
    next_seq: u64,
}

impl DebugRing {
    /// 以数据字节上限构造环形缓冲。
    pub fn new(byte_limit: usize) -> Self {
        Self {
            byte_limit,
            bytes: 0,
            frames: VecDeque::new(),
            next_seq: 1,
        }
    }

    /// 追加一帧；若数据字节总数超过上限，逐帧丢弃最旧的。
    pub fn push(&mut self, frame: DebugFrame) {
        self.bytes += frame.size;
        self.frames.push_back(frame);
        while self.bytes > self.byte_limit && self.frames.len() > 1 {
            if let Some(old) = self.frames.pop_front() {
                self.bytes -= old.size;
            }
        }
    }

    /// 取号并追加一帧（同锁原子）：分配 `next_seq`、构造并 push，返回该帧。
    ///
    /// 读/写线程并发调用时，帧的 `seq` 与插入顺序保持一致，避免「先取号后 push」
    /// 造成的乱序。
    pub fn push_frame(&mut self, direction: &str, ts_ms: u64, data: Vec<u8>) -> DebugFrame {
        let frame = DebugFrame::new(direction, self.next_seq, ts_ms, data);
        self.next_seq += 1;
        self.push(frame.clone());
        frame
    }

    /// 按插入顺序返回当前保留的全部帧。
    pub fn snapshot(&self) -> Vec<DebugFrame> {
        self.frames.iter().cloned().collect()
    }
}
