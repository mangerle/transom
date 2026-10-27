//! ZMODEM 字节流增量解析：帧头扫描器与数据子包扫描器。
//!
//! 两者均为"逐字节喂入、产出事件"的纯状态机，跨 TCP/串口数据块边界安全，
//! 由 attach 监督层驱动。语法严格对齐 lrzsz `zgethdr`/`zrdata`：
//! - 帧头前导为 `ZPAD ZPAD ZDLE <格式>`，ZPAD 兼容高位变体 0xAA，前导与
//!   hex 阶段按 noxrd7 语义剥离高位并丢弃流控字节；
//! - 子包数据按 zdlread 语义解码：裸 0x68-0x6B（h/i/j/k）是普通数据，只有
//!   ZDLE 前缀后的 0x68-0x6B 才是终止符——这是互操作正确性的关键定案；
//! - CRC 采用"传输值 == 明文 CRC"的等价判定，与 lrzsz 的零残差校验一致；
//! - 取消判定：任意位置连续 5 个 CAN。

use super::frames::{
    CAN_COUNT_CANCEL, ZBIN, ZBIN32, ZDLE, ZHEX, ZPAD, ZPAD_PARITY, ZRUB0, ZRUB1, crc16, crc32,
};

/// 子包数据累计上限。协议规定单子包不超过 1024 字节，转义后膨胀至多 2 倍，
/// 再留一倍余量；超出即判定为线路失控，防止恶意流撑爆内存。
pub(crate) const MAX_SUBPACKET_DATA: usize = 8192;

/// 解析出的帧头。`body` 保存线上四字节（body[0] 为位置最低字节，body[3]
/// 为 F0 标志字节），保持与 lrzsz hdr 数组相同的索引语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameHeader {
    pub kind: u8,
    pub body: [u8; 4],
    /// 头以 bin32 格式接收，指示后续子包使用 CRC-32。
    pub crc32_mode: bool,
}

impl FrameHeader {
    /// 恢复 32 位位置/偏移（线上低字节在前）。
    pub(crate) fn position(&self) -> u32 {
        u32::from_le_bytes(self.body)
    }

    /// F0 标志字节（能力/转换选项所在）。
    pub(crate) fn flags(&self) -> u8 {
        self.body[3]
    }
}

/// 帧头扫描器产出的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderEvent {
    Header(FrameHeader),
    /// 检测到对端取消序列。
    Cancel,
}

/// 数据子包扫描器产出的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubpacketEvent {
    /// 校验通过的子包：解转义后的数据与终止符原值（0x68-0x6B）。
    Packet(Vec<u8>, u8),
    /// CRC 校验失败或协议违规。
    Bad,
    /// 检测到对端取消序列。
    Cancel,
}

/// 十六进制半字节解码（兼容大小写；lrzsz 仅小写，放宽以容错）。
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// noxrd7 语义：剥离高位、丢弃 XON/XOFF，返回规范化字节。
fn strip_parity(byte: u8) -> Option<u8> {
    match byte & 0x7F {
        0x11 | 0x13 => None,
        stripped => Some(stripped),
    }
}

/// zdlread 语义单字节解码结果。
pub(crate) enum ZdleDecoded {
    Byte(u8),
    /// 子包终止符（仅转义上下文中出现，0x68-0x6B）。
    Terminator(u8),
    /// 流控字节或转义起始字节，已消化。
    Consumed,
    /// 协议违规。
    Violation,
}

/// ZDLE 转义上下文解码器：跨字节保持转义态。
///
/// 供二进制帧头体与数据子包共用。转义目标字节依 lrzsz `zdlread2`：
/// `0x40-0x5F`（含 0xC0-0xDF 高位变体）异或 0x40 还原控制字节，`l/m` 还原
/// 0x7F/0xFF，`h/i/j/k` 为终止符，流控字节丢弃后维持转义态继续等待。连续
/// CAN 的取消判定不在本层（由扫描器顶层统一统计）。
struct ZdleStream {
    escaped: bool,
}

impl ZdleStream {
    fn new() -> Self {
        Self { escaped: false }
    }

    fn feed(&mut self, byte: u8) -> ZdleDecoded {
        if self.escaped {
            self.escaped = false;
            return match byte {
                ZRUB0 => ZdleDecoded::Byte(0x7F),
                ZRUB1 => ZdleDecoded::Byte(0xFF),
                ZCRCE..=ZCRCW_LAST => ZdleDecoded::Terminator(byte),
                b if b & 0x60 == 0x40 => ZdleDecoded::Byte(b ^ 0x40),
                0x11 | 0x91 | 0x13 | 0x93 => {
                    self.escaped = true;
                    ZdleDecoded::Consumed
                }
                _ => ZdleDecoded::Violation,
            };
        }
        match byte {
            ZDLE => {
                self.escaped = true;
                ZdleDecoded::Consumed
            }
            0x11 | 0x91 | 0x13 | 0x93 => ZdleDecoded::Consumed,
            _ => ZdleDecoded::Byte(byte),
        }
    }
}

/// 子包终止符区间上界（0x68-0x6B 连续）。
const ZCRCE: u8 = 0x68;
/// 子包终止符区间下界。
const ZCRCW_LAST: u8 = 0x6B;

/// 帧头扫描器：从原始字节流中提取帧头或取消序列。
pub(crate) struct HeaderScanner {
    state: HeaderState,
    zdle: ZdleStream,
    can_run: usize,
    /// 二进制体待收字节数（kind + 4 body + CRC）。
    binary_remaining: usize,
    raw: [u8; 5],
    raw_filled: usize,
    crc_bytes: [u8; 4],
    crc_filled: usize,
    crc32_mode: bool,
    hex_high: Option<u8>,
    hex_bytes: [u8; 7],
    hex_filled: usize,
}

enum HeaderState {
    /// 寻找 ZPAD 前导。
    Seek,
    /// 已见 ZPAD，跳过冗余 ZPAD 与流控字节，等待 ZDLE。
    PadZdle,
    /// ZDLE 已见，读取格式字节。
    Format,
    /// 二进制体接收中。
    Binary,
    /// 十六进制体接收中。
    Hex,
    /// hex CRC 已收齐，等待 CR 确认。
    ExpectCr,
}

impl HeaderScanner {
    pub(crate) fn new() -> Self {
        Self {
            state: HeaderState::Seek,
            zdle: ZdleStream::new(),
            can_run: 0,
            binary_remaining: 0,
            raw: [0; 5],
            raw_filled: 0,
            crc_bytes: [0; 4],
            crc_filled: 0,
            crc32_mode: false,
            hex_high: None,
            hex_bytes: [0; 7],
            hex_filled: 0,
        }
    }

    /// 喂入一个原始字节，返回产出的事件（多数调用返回 None）。
    pub(crate) fn feed(&mut self, byte: u8) -> Option<HeaderEvent> {
        // 取消判定贯穿全部状态：ZDLE/CAN 同为 0x18，连续 5 个即取消
        //（含 lrzsz 的 ZDLE+4×CAN gotcan 路径）。
        if byte == ZDLE {
            self.can_run += 1;
            if self.can_run >= CAN_COUNT_CANCEL {
                self.reset();
                return Some(HeaderEvent::Cancel);
            }
        } else {
            self.can_run = 0;
        }
        match self.state {
            HeaderState::Seek => {
                if byte == ZPAD || byte == ZPAD_PARITY {
                    self.state = HeaderState::PadZdle;
                }
                None
            }
            HeaderState::PadZdle => self.feed_pad_zdle(byte),
            HeaderState::Format => self.feed_format(byte),
            HeaderState::Binary => self.feed_binary(byte),
            HeaderState::Hex => self.feed_hex(byte),
            HeaderState::ExpectCr => self.feed_expect_cr(byte),
        }
    }

    /// 复位到寻帧状态（错误或事件产出后调用）。
    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }

    fn feed_pad_zdle(&mut self, byte: u8) -> Option<HeaderEvent> {
        let stripped = strip_parity(byte)?;
        match stripped {
            // 冗余 ZPAD：继续等待 ZDLE。
            0x2A => {}
            ZDLE => self.state = HeaderState::Format,
            _ => self.state = HeaderState::Seek,
        }
        None
    }

    fn feed_format(&mut self, byte: u8) -> Option<HeaderEvent> {
        let stripped = strip_parity(byte)?;
        match stripped {
            ZBIN => self.begin_binary(false, 2),
            ZBIN32 => self.begin_binary(true, 4),
            ZHEX => {
                self.state = HeaderState::Hex;
                self.hex_high = None;
                self.hex_bytes = [0; 7];
                self.hex_filled = 0;
            }
            // 连续 ZDLE 属于取消计数路径（ZdleStream 已统计），此处忽略。
            ZDLE => {}
            _ => self.reset(),
        }
        None
    }

    fn begin_binary(&mut self, crc32_mode: bool, crc_len: usize) {
        self.state = HeaderState::Binary;
        self.crc32_mode = crc32_mode;
        self.binary_remaining = 5 + crc_len;
        self.raw = [0; 5];
        self.raw_filled = 0;
        self.crc_bytes = [0; 4];
        self.crc_filled = 0;
        self.zdle = ZdleStream::new();
    }

    fn feed_binary(&mut self, byte: u8) -> Option<HeaderEvent> {
        match self.zdle.feed(byte) {
            ZdleDecoded::Byte(value) => {
                if self.raw_filled < 5 {
                    self.raw[self.raw_filled] = value;
                    self.raw_filled += 1;
                } else {
                    self.crc_bytes[self.crc_filled] = value;
                    self.crc_filled += 1;
                }
                self.binary_remaining = self.binary_remaining.saturating_sub(1);
                if self.binary_remaining == 0 {
                    return self.finish_binary();
                }
                None
            }
            ZdleDecoded::Terminator(_) | ZdleDecoded::Violation => {
                self.reset();
                None
            }
            ZdleDecoded::Consumed => None,
        }
    }

    fn finish_binary(&mut self) -> Option<HeaderEvent> {
        let kind = self.raw[0];
        let mut body = [0u8; 4];
        body.copy_from_slice(&self.raw[1..5]);
        let crc32_mode = self.crc32_mode;
        let valid = if crc32_mode {
            crc32(&[kind, body[0], body[1], body[2], body[3]]) == u32::from_le_bytes(self.crc_bytes)
        } else {
            crc16(&[kind, body[0], body[1], body[2], body[3]])
                == u16::from_be_bytes([self.crc_bytes[0], self.crc_bytes[1]])
        };
        self.reset();
        if valid {
            Some(HeaderEvent::Header(FrameHeader {
                kind,
                body,
                crc32_mode,
            }))
        } else {
            None
        }
    }

    /// 十六进制体：成对 hex 数字依序组成 kind、4 字节 body、CRC 高位、低位。
    fn feed_hex(&mut self, byte: u8) -> Option<HeaderEvent> {
        let stripped = strip_parity(byte)?;
        let Some(digit) = hex_digit(stripped) else {
            self.reset();
            return None;
        };
        let Some(high) = self.hex_high.take() else {
            self.hex_high = Some(digit);
            return None;
        };
        self.hex_bytes[self.hex_filled] = (high << 4) | digit;
        self.hex_filled += 1;
        if self.hex_filled == 7 {
            self.state = HeaderState::ExpectCr;
        }
        None
    }

    fn feed_expect_cr(&mut self, byte: u8) -> Option<HeaderEvent> {
        let stripped = strip_parity(byte)?;
        if stripped != 0x0D {
            self.reset();
            return None;
        }
        let kind = self.hex_bytes[0];
        let mut body = [0u8; 4];
        body.copy_from_slice(&self.hex_bytes[1..5]);
        let valid = crc16(&[kind, body[0], body[1], body[2], body[3]])
            == u16::from_be_bytes([self.hex_bytes[5], self.hex_bytes[6]]);
        self.reset();
        if valid {
            Some(HeaderEvent::Header(FrameHeader {
                kind,
                body,
                crc32_mode: false,
            }))
        } else {
            None
        }
    }
}

/// 数据子包扫描器：解码数据、识别终止符并校验 CRC。
///
/// CRC 宽度由 `set_crc32` 按最近接收帧头的格式动态切换（lrzsz 的 Crc32
/// 全局标志语义）：sz 声明 CANFC32 后使用 bin32 头与 CRC-32 子包，老实现
/// 则保持 CRC-16，两种必须同时支持。
pub(crate) struct SubpacketScanner {
    zdle: ZdleStream,
    can_run: usize,
    data: Vec<u8>,
    terminator: Option<u8>,
    crc_bytes: [u8; 4],
    crc_filled: usize,
    crc32_mode: bool,
}

impl SubpacketScanner {
    pub(crate) fn new() -> Self {
        Self {
            zdle: ZdleStream::new(),
            can_run: 0,
            data: Vec::with_capacity(1024),
            terminator: None,
            crc_bytes: [0; 4],
            crc_filled: 0,
            crc32_mode: false,
        }
    }

    /// 依据最近帧头格式切换子包 CRC 宽度。
    pub(crate) fn set_crc32(&mut self, enabled: bool) {
        self.crc32_mode = enabled;
    }

    /// 清空累计状态（文件切换、错误恢复时调用）。
    pub(crate) fn reset(&mut self) {
        self.zdle = ZdleStream::new();
        self.data.clear();
        self.terminator = None;
        self.crc_bytes = [0; 4];
        self.crc_filled = 0;
    }

    /// 喂入一个原始字节，返回产出的事件（多数调用返回 None）。
    pub(crate) fn feed(&mut self, byte: u8) -> Option<SubpacketEvent> {
        if byte == ZDLE {
            self.can_run += 1;
            if self.can_run >= CAN_COUNT_CANCEL {
                self.reset();
                return Some(SubpacketEvent::Cancel);
            }
        } else {
            self.can_run = 0;
        }
        match self.zdle.feed(byte) {
            ZdleDecoded::Byte(value) => {
                if let Some(terminator) = self.terminator {
                    self.crc_bytes[self.crc_filled] = value;
                    self.crc_filled += 1;
                    let need = if self.crc32_mode { 4 } else { 2 };
                    if self.crc_filled == need {
                        return Some(self.finish(terminator));
                    }
                    return None;
                }
                self.data.push(value);
                if self.data.len() > MAX_SUBPACKET_DATA {
                    self.reset();
                    return Some(SubpacketEvent::Bad);
                }
                None
            }
            ZdleDecoded::Terminator(value) => {
                self.terminator = Some(value);
                None
            }
            ZdleDecoded::Violation => {
                self.reset();
                Some(SubpacketEvent::Bad)
            }
            ZdleDecoded::Consumed => None,
        }
    }

    fn finish(&mut self, terminator: u8) -> SubpacketEvent {
        let mut input = std::mem::take(&mut self.data);
        input.push(terminator);
        let valid = if self.crc32_mode {
            crc32(&input) == u32::from_le_bytes(self.crc_bytes)
        } else {
            crc16(&input) == u16::from_be_bytes([self.crc_bytes[0], self.crc_bytes[1]])
        };
        let payload_len = input.len() - 1;
        self.reset();
        if valid {
            input.truncate(payload_len);
            SubpacketEvent::Packet(input, terminator)
        } else {
            SubpacketEvent::Bad
        }
    }
}

#[cfg(test)]
mod tests {
    include!("scanner_tests.rs");
}
