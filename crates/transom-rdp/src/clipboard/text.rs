//! 剪贴板文本编解码（UTF-16LE 与 ANSI 回退）及体积上限防护。
//!
//! 设计原理（Why）：
//! 1. Windows RDP 剪贴板主格式为 `CF_UNICODETEXT`（UTF-16LE + 双字节零终止符），
//!    部分老旧客户端/服务端仅提供 `CF_TEXT`（单字节文本）；两端均需支持；
//! 2. 解码使用 `String::from_utf16_lossy`，对孤立代理码元安全替换为 U+FFFD，
//!    避免单个异常码元导致整段文本被静默丢弃；
//! 3. 强制限制单次剪贴板文本最大载荷为 [`MAX_CLIPBOARD_BYTES`]（2 MiB），防止远端
//!    误复制超大文本触发客户端内存暴涨。

/// 单次剪贴板文本最大允许字节数（2 MiB）。
pub const MAX_CLIPBOARD_BYTES: usize = 2 * 1024 * 1024;

/// 将 Rust UTF-8 字符串编码为 `CF_UNICODETEXT`（UTF-16LE 含末尾 null 终止符）。
pub fn encode_text_payload(text: &str) -> Vec<u8> {
    let bounded = truncate_utf8_bytes(text, MAX_CLIPBOARD_BYTES / 2);
    let utf16_units: Vec<u16> = bounded.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = Vec::with_capacity(utf16_units.len() * 2);
    for unit in utf16_units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// 将 Rust UTF-8 字符串编码为 `CF_TEXT`（单字节零终止回退格式）。
pub fn encode_ansi_payload(text: &str) -> Vec<u8> {
    let bounded = truncate_utf8_bytes(text, MAX_CLIPBOARD_BYTES.saturating_sub(1));
    let mut bytes = Vec::with_capacity(bounded.len() + 1);
    bytes.extend_from_slice(bounded.as_bytes());
    bytes.push(0);
    bytes
}

/// 将来自远端的 `CF_UNICODETEXT`（UTF-16LE）载荷解码为 UTF-8 字符串。
pub fn decode_text_payload(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }

    let capped = &bytes[..bytes.len().min(MAX_CLIPBOARD_BYTES)];
    let mut units = Vec::with_capacity(capped.len() / 2);

    for chunk in capped.as_chunks::<2>().0 {
        let unit = u16::from_le_bytes(*chunk);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }

    Some(String::from_utf16_lossy(&units))
}

/// 将来自远端的 `CF_TEXT`（单字节零终止）载荷解码为 UTF-8 字符串。
pub fn decode_ansi_payload(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let capped = &bytes[..bytes.len().min(MAX_CLIPBOARD_BYTES)];
    let end = capped.iter().position(|&b| b == 0).unwrap_or(capped.len());
    Some(String::from_utf8_lossy(&capped[..end]).into_owned())
}

fn truncate_utf8_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_utf16_replaces_unpaired_surrogate_instead_of_dropping() {
        // 合法字符 'A' + 孤立高代理 0xD800 + 终止符
        let raw = [0x41, 0x00, 0x00, 0xD8, 0x00, 0x00];
        let decoded = decode_text_payload(&raw).expect("应宽容解码");
        assert_eq!(decoded, "A\u{FFFD}");
    }

    #[test]
    fn ansi_payload_roundtrip() {
        let encoded = encode_ansi_payload("hello rdp");
        assert_eq!(encoded.last(), Some(&0));
        let decoded = decode_ansi_payload(&encoded).expect("应成功解码 ANSI");
        assert_eq!(decoded, "hello rdp");
    }
}
