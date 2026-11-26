//! ESC 引导协议序列的通用增量扫描器（OSC 与 Kitty APC 共用）。
//!
//! 设计原理（Why）：两种协议的边界规则完全相同——等待 `ESC` 与引导字节、
//! 收集载荷、以 `BEL` 或 `ESC \` 收尾、超长丢弃。此前两份扫描器各自维护
//! 四个互斥布尔与逐字节状态机，任何边界修复都要同步两处。这里只保留
//! “如何界定一条序列”，载荷语义仍由调用方解析。
//!
//! 代价与局限：扫描器不理解序列类型，引导字节与载荷上限由构造参数决定；
//! 解析失败或未知序列由回调自行忽略，扫描器不做语义判断。

const ESCAPE: u8 = 0x1b;
const BEL: u8 = 0x07;
const ST_TAIL: u8 = 0x5c;

/// 单条序列的增量扫描状态。
pub(crate) struct SequenceScanner {
    /// 引导字节（OSC 为 `]`，Kitty APC 为 `_`）。
    intro: u8,
    max_payload: usize,
    /// 上一个字节是 `ESC`，正在确认它后面是不是引导字节。
    awaiting_intro: bool,
    /// 正在收集载荷。
    in_payload: bool,
    /// 载荷里已见 `ESC`，下一个字节是 `\` 才算 ST 结束。
    awaiting_st: bool,
    /// 载荷已超长，丢弃内容直到序列结束。
    overflow: bool,
    payload: Vec<u8>,
}

impl SequenceScanner {
    pub(crate) fn new(intro: u8, max_payload: usize) -> Self {
        Self {
            intro,
            max_payload,
            awaiting_intro: false,
            in_payload: false,
            awaiting_st: false,
            overflow: false,
            payload: Vec::new(),
        }
    }

    /// 喂入一段字节流，返回识别到的完整序列及其末字节偏移。
    ///
    /// `on_sequence` 在序列收尾时被调用；超长序列不会触发回调。
    pub(crate) fn feed_with_offsets<T>(
        &mut self,
        bytes: &[u8],
        mut on_sequence: impl FnMut(&[u8], usize, &mut Vec<(usize, T)>),
    ) -> Vec<(usize, T)> {
        let mut events = Vec::new();
        for (offset, &byte) in bytes.iter().enumerate() {
            self.push(byte, offset, &mut events, &mut on_sequence);
        }
        events
    }

    fn push<T>(
        &mut self,
        byte: u8,
        offset: usize,
        events: &mut Vec<(usize, T)>,
        on_sequence: &mut impl FnMut(&[u8], usize, &mut Vec<(usize, T)>),
    ) {
        if !self.in_payload {
            self.push_idle(byte);
            return;
        }
        if self.awaiting_st {
            self.awaiting_st = false;
            if byte == ST_TAIL || byte == BEL {
                self.close(offset, events, on_sequence);
            } else {
                // ESC 后不是 ST：本身属于载荷内容。
                self.collect(ESCAPE);
                self.collect(byte);
            }
            return;
        }
        match byte {
            BEL => self.close(offset, events, on_sequence),
            ESCAPE => self.awaiting_st = true,
            other => self.collect(other),
        }
    }

    fn push_idle(&mut self, byte: u8) {
        if self.awaiting_intro {
            self.awaiting_intro = false;
            if byte == self.intro {
                self.in_payload = true;
                self.payload.clear();
                self.overflow = false;
            } else if byte == ESCAPE {
                self.awaiting_intro = true;
            }
            return;
        }
        if byte == ESCAPE {
            self.awaiting_intro = true;
        }
    }

    fn collect(&mut self, byte: u8) {
        if self.overflow {
            return;
        }
        self.payload.push(byte);
        if self.payload.len() > self.max_payload {
            self.payload.clear();
            self.overflow = true;
        }
    }

    fn close<T>(
        &mut self,
        offset: usize,
        events: &mut Vec<(usize, T)>,
        on_sequence: &mut impl FnMut(&[u8], usize, &mut Vec<(usize, T)>),
    ) {
        let overflowed = self.overflow;
        self.in_payload = false;
        self.awaiting_st = false;
        self.overflow = false;
        if overflowed {
            self.payload.clear();
            return;
        }
        on_sequence(&self.payload, offset, events);
        self.payload.clear();
    }
}
