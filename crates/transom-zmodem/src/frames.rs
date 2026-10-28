//! ZMODEM 帧层编解码：常量、CRC、ZDLE 转义与三种帧头/子包的构造。
//!
//! 全部字节级语义以 lrzsz 参考实现为准（zm.c/zmodem.h/zglobal.h），而非仅凭
//! zmodem.txt 规范文本，避免实现分歧导致的互操作失败。关键定案：
//! - 帧头线上字节序为 `type, F3, F2, F1, F0`，其中 F3 是位置的最低字节
//!   （小端），F0 是能力标志字节，最后发送；
//! - 十六进制帧头的 CRC 数字为小写，高位字节在前，尾部以 CR LF 收尾；
//! - CRC-16 为 CCITT/XMODEM（poly 0x1021、初值 0、MSB 先行），CRC-32 为
//!   标准反射算法，线路上低字节在前；
//! - ZDLE 转义范围与 lrzsz 默认 `zsendline` 表一致（含 @-CR 规则）。

/// ZPAD：帧头前导字节（`*`）。
pub(crate) const ZPAD: u8 = 0x2A;
/// ZPAD 的高位变体（0xAA）：远端线路奇偶位污染时仍可识别帧头。
pub(crate) const ZPAD_PARITY: u8 = 0xAA;
/// ZDLE：转义定界字节（CAN，Ctrl-X）。
pub(crate) const ZDLE: u8 = 0x18;
/// 二进制帧头格式标记（CRC-16）。
pub(crate) const ZBIN: u8 = 0x41;
/// 十六进制帧头格式标记（CRC-16）。
pub(crate) const ZHEX: u8 = 0x42;
/// 二进制帧头格式标记（CRC-32）。
pub(crate) const ZBIN32: u8 = 0x43;

/// 帧类型：请求启动（sz 侧发起）。
pub(crate) const ZRQINIT: u8 = 0;
/// 帧类型：接收方能力宣告。
pub(crate) const ZRINIT: u8 = 1;
/// 帧类型：发送方转义与 Attn 序列宣告。
pub(crate) const ZSINIT: u8 = 2;
/// 帧类型：确认应答。
pub(crate) const ZACK: u8 = 3;
/// 帧类型：文件开始（携带文件属性）。
pub(crate) const ZFILE: u8 = 4;
/// 帧类型：拒绝接收当前文件。
pub(crate) const ZSKIP: u8 = 5;
/// 帧类型：否定应答。
pub(crate) const ZNAK: u8 = 6;
/// 帧类型：会话收尾。
pub(crate) const ZFIN: u8 = 8;
/// 帧类型：请求从指定偏移恢复传输。
pub(crate) const ZRPOS: u8 = 9;
/// 帧类型：文件数据帧头。
pub(crate) const ZDATA: u8 = 10;
/// 帧类型：文件数据发送完毕。
pub(crate) const ZEOF: u8 = 11;

/// 子包终止符：数据块结束（期望接收方继续读下一帧头）。
pub(crate) const ZCRCE: u8 = 0x68;
/// 子包终止符：数据块继续，无需应答。
pub(crate) const ZCRCG: u8 = 0x69;
/// 子包终止符：数据块继续，要求 ZACK 应答。
pub(crate) const ZCRCQ: u8 = 0x6A;
/// 子包终止符：数据块继续，要求 ZACK 应答后才发下一帧。
pub(crate) const ZCRCW: u8 = 0x6B;
/// 转义字节：还原为 0x7F（删除键）。
pub(crate) const ZRUB0: u8 = 0x6C;
/// 转义字节：还原为 0xFF。
pub(crate) const ZRUB1: u8 = 0x6D;

/// ZRINIT 能力：可全双工（必须声明，sz 依赖它）。
pub(crate) const CANFDX: u8 = 0x01;
/// ZRINIT 能力：接收缓冲可覆盖。
pub(crate) const CANOVIO: u8 = 0x02;
/// ZRINIT 能力：支持 CRC-32 子包与 bin32 帧头。
pub(crate) const CANFC32: u8 = 0x20;

/// 接收方判定取消序列所需的连续 CAN 字节数（lrzsz zgethdr 阈值）。
pub(crate) const CAN_COUNT_CANCEL: usize = 5;
/// 取消序列中 CAN 后缀随附的退格字节数（lrzsz canistr 取 10）。
pub(crate) const CANCEL_BS_COUNT: usize = 10;

/// 构造接收方 ZRINIT 的能力标志：可全双工 + 可覆盖 + CRC-32。
///
/// 设计原理：声明 CANFC32 让 sz 走 CRC-32（速率高、校验强）；lrzsz 的 rz
/// 默认同样声明 CANFDX|CANOVIO|CANFC32，保持行为对齐。
pub(crate) fn receiver_flags() -> u8 {
    CANFDX | CANOVIO | CANFC32
}

/// CRC-16/XMODEM：poly 0x1021、初值 0、MSB 先行、无终值异或。
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// 标准 CRC-32（反射，poly 0xEDB88320，初值/终值均取反）。
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// lrzsz 默认 `zsendline` 的 ZDLE 转义判定：需要转义返回真。
///
/// 定案依据（zsendline_tab 默认表）：控制字节 0x10/0x11/0x13 及其 0x80 变体、
/// ZDLE 本身必须转义；CR（0x0D/0x8D）仅在前一输出字节为 `@` 时转义（防止
/// `@\r` 被老实现误判为校验序列）；其余字节字面发送。
fn zdle_needed(byte: u8, last_sent: Option<u8>) -> bool {
    match byte {
        ZDLE | 0x10 | 0x90 | 0x11 | 0x91 | 0x13 | 0x93 => true,
        0x0D | 0x8D => matches!(last_sent, Some(0x40) | Some(0xC0)),
        _ => false,
    }
}

/// 把原始字节按 ZDLE 规则转义后追加到输出缓冲。
///
/// `last_sent` 由调用方传入跨调用延续的"上一输出字节"状态（转义后的裸字节
/// 语义），用于 @-CR 规则；函数返回新的 last_sent，供流式发送循环串联。
pub(crate) fn escape_zdle_into(
    input: &[u8],
    out: &mut Vec<u8>,
    last_sent: Option<u8>,
) -> Option<u8> {
    let mut last = last_sent;
    for &byte in input {
        if zdle_needed(byte, last) {
            out.push(ZDLE);
            out.push(byte ^ 0x40);
        } else {
            out.push(byte);
        }
        last = Some(byte);
    }
    last
}

/// 便捷封装：一次性转义完整缓冲（无跨调用状态）。
#[cfg(test)]
pub(crate) fn escape_zdle(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + 8);
    escape_zdle_into(input, &mut out, None);
    out
}

/// 构造十六进制帧头：`ZPAD ZPAD ZDLE ZHEX` + 小写 hex(type) + hex(F3..F0)
/// + hex(CRC16 高位) + hex(CRC16 低位) + CR LF。
///
/// 采用 hex 而非二进制头发送所有控制帧：hex 对线路透明度最高（任意奇偶位、
/// 流控注入都不破坏语义），且所有实现必须支持；性能差异仅在头帧（几十字节）。
pub(crate) fn build_hex_header(kind: u8, body: [u8; 4]) -> Vec<u8> {
    let mut crc_input = Vec::with_capacity(5);
    crc_input.push(kind);
    crc_input.extend_from_slice(&body);
    let crc = crc16(&crc_input);
    let mut out = Vec::with_capacity(24);
    out.extend_from_slice(&[ZPAD, ZPAD, ZDLE, ZHEX]);
    for byte in [
        kind,
        body[0],
        body[1],
        body[2],
        body[3],
        (crc >> 8) as u8,
        crc as u8,
    ] {
        let digits = b"0123456789abcdef";
        out.push(digits[usize::from(byte >> 4)]);
        out.push(digits[usize::from(byte & 0x0F)]);
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// 构造二进制帧头（CRC-16）。生产路径接收方统一以 hex 头应答、发送方
/// 以 hex/bin32 发起，二进制 16 头仅在对端能力受限的解析场景保留，
/// 因此构造器只在测试中构造样本使用。
#[cfg(test)]
pub(crate) fn build_bin16_header(kind: u8, body: [u8; 4]) -> Vec<u8> {
    let mut crc_input = Vec::with_capacity(5);
    crc_input.push(kind);
    crc_input.extend_from_slice(&body);
    let crc = crc16(&crc_input);
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&[ZPAD, ZPAD, ZDLE, ZBIN]);
    escape_zdle_into(
        &[
            kind,
            body[0],
            body[1],
            body[2],
            body[3],
            (crc >> 8) as u8,
            crc as u8,
        ],
        &mut out,
        None,
    );
    out
}

/// 构造二进制帧头（CRC-32）。
pub(crate) fn build_bin32_header(kind: u8, body: [u8; 4]) -> Vec<u8> {
    let mut crc_input = Vec::with_capacity(5);
    crc_input.push(kind);
    crc_input.extend_from_slice(&body);
    // crc32 自身已含末位取反（初值 0xFFFFFFFF 的标准 CRC-32），线上就发这个
    // 值；lrzsz `zc3send` 的 `crc = ~crc` 即此步，再取反一次会退化成裸累加值，
    // 真实 sz/rz 会把每个 CRC-32 帧判为坏帧。
    let crc = crc32(&crc_input);
    let mut out = Vec::with_capacity(20);
    out.extend_from_slice(&[ZPAD, ZPAD, ZDLE, ZBIN32]);
    let mut last = escape_zdle_into(&[kind, body[0], body[1], body[2], body[3]], &mut out, None);
    // CRC-32 线上低字节在前；高位字（bit5/6 置位）可字面发送，此处统一走
    // 转义通道：接收方 zdlread 两条路径等价，转义仅稍增字节量，换取实现简单。
    for shift in [0u32, 8, 16, 24] {
        let byte = (crc >> shift) as u8;
        last = escape_zdle_into(&[byte], &mut out, last);
    }
    out
}

/// 构造数据子包：转义数据 + `ZDLE` + 终止符 + 转义 CRC-16。
///
/// CRC 覆盖"数据 + 终止符"，与 lrzsz `zsdata` 一致；终止符始终以 ZDLE
/// 前缀发送，这是接收方区分数据中 `h/i/j/k` 字节与真实终止符的唯一依据。
pub(crate) fn build_subpacket16(data: &[u8], terminator: u8) -> Vec<u8> {
    let mut crc_input = Vec::with_capacity(data.len() + 1);
    crc_input.extend_from_slice(data);
    crc_input.push(terminator);
    let crc = crc16(&crc_input);

    let mut out = Vec::with_capacity(data.len() + 12);
    escape_zdle_into(data, &mut out, None);
    out.push(ZDLE);
    out.push(terminator);
    let crc_bytes = [(crc >> 8) as u8, crc as u8];
    escape_zdle_into(&crc_bytes, &mut out, Some(terminator));
    out
}

/// 构造数据子包（CRC-32 校验）。
pub(crate) fn build_subpacket32(data: &[u8], terminator: u8) -> Vec<u8> {
    let mut crc_input = Vec::with_capacity(data.len() + 1);
    crc_input.extend_from_slice(data);
    crc_input.push(terminator);
    let crc = crc32(&crc_input);

    let mut out = Vec::with_capacity(data.len() + 16);
    escape_zdle_into(data, &mut out, None);
    out.push(ZDLE);
    out.push(terminator);
    let mut last = Some(terminator);
    for shift in [0u32, 8, 16, 24] {
        let byte = (crc >> shift) as u8;
        last = escape_zdle_into(&[byte], &mut out, last);
    }
    out
}

/// 发送方取消序列：CAN ×8 + BS ×10（lrzsz canistr 的前半段）。
pub(crate) fn cancel_sequence() -> Vec<u8> {
    let mut out = vec![ZDLE; 8];
    out.extend(std::iter::repeat_n(0x08, CANCEL_BS_COUNT));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_known_vectors() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // 真实抓包：rz 的 ZRINIT hex 头 `**^XB0100000023` 后随 CRC hex "be50"。
        assert_eq!(crc16(&[ZRINIT, 0, 0, 0, 0x23]), 0xBE50);
        // ZRQINIT 全零体 CRC 为 0，抓包中表现为连续 "0000"。
        assert_eq!(crc16(&[ZRQINIT, 0, 0, 0, 0]), 0x0000);
        assert_eq!(crc32(&[ZFILE, 0, 0, 0, 0]), 0x33A2_51DD);
    }

    /// 线上格式的独立基准测试：期望值由外部实现（zlib.crc32）算出，而非本模块
    /// 自算，否则编解码两侧同时写错（例如 CRC-32 再取一次反）仍能自洽全绿，
    /// 真机 sz/rz 却把每个 CRC-32 帧判为坏帧。字节序定案依据为 Forsberg 原始
    /// zsend.c：16 位 `zsendline(crc>>8); zsendline(crc)` 高位先行，
    /// 32 位 `crc=~crc` 后逐字节右移，即标准值且低位先行。
    #[test]
    fn wire_crc_matches_external_reference_vectors() {
        let header = build_bin32_header(ZFILE, [0, 0, 0, 0]);
        assert!(
            header.ends_with(&0x33A2_51DDu32.to_le_bytes()),
            "CRC-32 帧头须发标准值且低位在前，实际线上尾部: {header:?}"
        );
        let subpacket = build_subpacket32(b"hello", ZCRCG);
        assert!(
            subpacket.ends_with(&0xE2_81_00_C2u32.to_le_bytes()),
            "CRC-32 子包须发标准值且低位在前，实际线上尾部: {subpacket:?}"
        );
        let header16 = build_bin16_header(ZFILE, [0, 0, 0, 0]);
        assert!(
            header16
                .as_slice()
                .ends_with(&crc16(&[ZFILE, 0, 0, 0, 0]).to_be_bytes()),
            "CRC-16 帧头须高位先行"
        );
    }

    #[test]
    fn hex_header_roundtrip_layout() {
        let head = build_hex_header(ZRINIT, [0, 0, 0, 0x23]);
        assert_eq!(&head[..4], &[ZPAD, ZPAD, ZDLE, ZHEX]);
        let text = String::from_utf8(head[4..head.len() - 2].to_vec()).expect("hex 头必为 ASCII");
        assert_eq!(text, "0100000023be50");
        assert!(head.ends_with(b"\r\n"));
    }

    #[test]
    fn zdle_escape_table_matches_lrzsz_defaults() {
        let escaped = escape_zdle(&[ZDLE, 0x11, 0x91, 0x13, 0x93, 0x10, 0x90, 0x41]);
        assert_eq!(
            escaped,
            vec![
                ZDLE, 0x58, ZDLE, 0x51, ZDLE, 0xD1, ZDLE, 0x53, ZDLE, 0xD3, ZDLE, 0x50, ZDLE, 0xD0,
                0x41
            ]
        );
        // 普通字节不做任何转义。
        assert_eq!(escape_zdle(b"abc123"), b"abc123");
    }

    #[test]
    fn zdle_escapes_cr_after_at_only() {
        // `@` 后的 CR 必须转义，防止 `@\r` 序列歧义。
        assert_eq!(escape_zdle(&[0x40, 0x0D]), vec![0x40, ZDLE, 0x4D]);
        assert_eq!(escape_zdle(&[0x41, 0x0D]), vec![0x41, 0x0D]);
        // 跨调用延续 last_sent：上段结尾是 0x40，下一段开头的 CR 仍需转义。
        let mut out = escape_zdle(&[0x40]);
        escape_zdle_into(&[0x0D], &mut out, Some(0x40));
        assert_eq!(out, vec![0x40, ZDLE, 0x4D]);
    }

    #[test]
    fn binary_header_contains_body_and_crc() {
        let head = build_bin16_header(ZRPOS, [0, 0, 0, 0]);
        assert_eq!(&head[..4], &[ZPAD, ZPAD, ZDLE, ZBIN]);
        // 全零体无控制字节，尾部两个字节即 CRC16(ZRPOS,0)=0xa87c。
        assert_eq!(&head[head.len() - 2..], &[0xA8, 0x7C]);
        let head32 = build_bin32_header(ZFILE, [0, 0, 0, 0]);
        assert_eq!(&head32[..4], &[ZPAD, ZPAD, ZDLE, ZBIN32]);
    }

    #[test]
    fn subpacket_terminator_is_zdle_prefixed() {
        let packet = build_subpacket16(b"hello", ZCRCW);
        // 数据无控制字节时线路上应出现 0x18 0x6B 的终止符对。
        let marker = packet
            .windows(2)
            .position(|w| w == [ZDLE, ZCRCW])
            .expect("缺少终止符对");
        assert_eq!(&packet[..marker], b"hello");
        // 终止符之后是两个 CRC 字节（0x56 0xe2 无需转义）。
        assert_eq!(&packet[marker + 2..], &[0x56, 0xE2]);
    }

    #[test]
    fn subpacket32_little_endian_crc() {
        let packet = build_subpacket32(b"hello", ZCRCW);
        let marker = packet
            .windows(2)
            .position(|w| w == [ZDLE, ZCRCW])
            .expect("缺少终止符对");
        // 标准 CRC-32（含末位取反）低字节先行；本例 4 个 CRC 字节均无需转义。
        assert_eq!(&packet[marker + 2..], &[0xEE, 0x61, 0x8F, 0x0C]);
    }

    #[test]
    fn cancel_sequence_layout() {
        let seq = cancel_sequence();
        assert_eq!(&seq[..8], &[ZDLE; 8]);
        assert_eq!(&seq[8..], &[0x08; CANCEL_BS_COUNT]);
    }
}
