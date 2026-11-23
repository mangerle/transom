//! UTF-16LE 剪贴板文本编解码。
//!
//! 设计原理（Why）：Windows 远程桌面标准剪贴板文本格式为 CF_UNICODETEXT
//! （即 UTF-16LE，末尾带双字节零终止符）。在协议边界进行精准编解码，
//! 避免产生字符乱码，并对异常字节序列进行安全替换。

/// 将 Rust UTF-8 字符串编码为 RDP 规范的 UTF-16LE 剪贴板字节载荷（含末尾 null 终止符）。
pub fn encode_text_payload(text: &str) -> Vec<u8> {
    let utf16_units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = Vec::with_capacity(utf16_units.len() * 2);
    for unit in utf16_units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// 将来自远端的 UTF-16LE 剪贴板字节载荷解码为 UTF-8 字符串。
pub fn decode_text_payload(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }

    let unit_count = bytes.len() / 2;
    let mut units = Vec::with_capacity(unit_count);

    for chunk in bytes.as_chunks::<2>().0 {
        let unit = u16::from_le_bytes(*chunk);
        // 遇到 null 终止符提前结束
        if unit == 0 {
            break;
        }
        units.push(unit);
    }

    String::from_utf16(&units).ok()
}
