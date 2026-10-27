//! 自定义终端按键映射与转义序列编解码。

use serde::{Deserialize, Serialize};

/// 自定义终端按键映射：把本地键盘组合映射为远端程序期望的转义序列。
/// - 终端程序（尤其是远程 TUI）需要一些本地键盘发不出的序列，例如
///   `Ctrl+Shift+Left` 在 tmux/vim 里应发 `\e[1;6D`；让用户自己配置比在
///   代码里硬编码某套终端约定更可靠；
/// - `keys` 使用 gpui 的按键语法（`ctrl-shift-left`、`alt-backspace`、`f1`），
///   `sequence` 支持 `\e`、`\x1b`、`\n`、`\r`、`\t`、`\\`、`\0` 与 `\u{...}`；
/// - 校验放在设置保存时一次性完成，运行期只做查表匹配，键盘热路径不做解析。
///
/// 代价与局限：映射在应用快捷键之后、终端默认编码之前生效；与全局应用
/// 快捷键冲突时以全局快捷键为准（避免用户把设置页锁死）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalKeyBinding {
    /// gpui 按键语法，如 `ctrl-shift-left`。
    pub keys: String,
    /// 要发送的原始字节序列（支持反斜杠转义）。
    pub sequence: String,
}

impl TerminalKeyBinding {
    /// 解析多行配置，每行形如 `ctrl-shift-left = \e[1;6D`。
    ///
    /// 空行与 `#` 开头的注释忽略；错误信息带行号，便于在设置页定位。
    pub fn parse_multiline(text: &str) -> Result<Vec<Self>, String> {
        let mut bindings = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((keys, sequence)) = line.split_once('=') else {
                return Err(format!(
                    "第 {} 行缺少 \"=\"，格式应为：按键 = 转义序列",
                    index + 1
                ));
            };
            let keys = keys.trim();
            let sequence = sequence.trim();
            if keys.is_empty() {
                return Err(format!("第 {} 行按键为空", index + 1));
            }
            if sequence.is_empty() {
                return Err(format!("第 {} 行转义序列为空", index + 1));
            }
            decode_escape_sequence(sequence)
                .map_err(|error| format!("第 {} 行转义序列非法: {error}", index + 1))?;
            bindings.push(Self {
                keys: keys.to_string(),
                sequence: sequence.to_string(),
            });
        }
        Ok(bindings)
    }

    /// 序列化为多行配置文本；与 [`Self::parse_multiline`] 互逆。
    pub fn to_multiline(bindings: &[Self]) -> String {
        bindings
            .iter()
            .map(|binding| format!("{} = {}", binding.keys, binding.sequence))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 解码为实际发送的字节；供终端视图与设置校验共用。
    ///
    /// # Errors
    /// 转义不合法、`\x` 后不是两位十六进制或序列为空时返回中文原因。
    pub fn decode_sequence(&self) -> Result<Vec<u8>, String> {
        decode_escape_sequence(&self.sequence)
    }
}

/// 解码带反斜杠转义的字节序列。
pub(crate) fn decode_escape_sequence(input: &str) -> Result<Vec<u8>, String> {
    fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let Some(escaped) = chars.next() else {
            return Err("序列以单个反斜杠结尾".to_string());
        };
        match escaped {
            'e' | 'E' => bytes.push(0x1b),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            't' => bytes.push(b'\t'),
            '0' => bytes.push(0),
            '\\' => bytes.push(b'\\'),
            'x' | 'X' => {
                let Some(high) = chars.next().and_then(|ch| hex_value(ch as u8)) else {
                    return Err("\\x 后必须跟两位十六进制数字".to_string());
                };
                let Some(low) = chars.next().and_then(|ch| hex_value(ch as u8)) else {
                    return Err("\\x 后必须跟两位十六进制数字".to_string());
                };
                bytes.push((high << 4) | low);
            }
            'u' => {
                let Some('{') = chars.next() else {
                    return Err("\\u 后必须跟 {码点}，例如 \\u{1b}".to_string());
                };
                let mut value = 0u32;
                let mut digits = 0usize;
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    let Some(digit) = hex_value(ch as u8) else {
                        return Err("\\u{...} 内只能是十六进制数字".to_string());
                    };
                    digits += 1;
                    if digits > 6 {
                        return Err("\\u{...} 码点过长".to_string());
                    }
                    value = value.saturating_mul(16).saturating_add(u32::from(digit));
                }
                if !closed || digits == 0 {
                    return Err("\\u{...} 未闭合或为空".to_string());
                }
                let Some(ch) = char::from_u32(value) else {
                    return Err(format!("\\u{{{value:x}}} 不是合法 Unicode 码点"));
                };
                let mut buffer = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            }
            other => {
                return Err(format!(
                    "不支持的转义 \\{other}，可用 \\e、\\x1b、\\n、\\r、\\t、\\\\、\\0、\\u{{1b}}"
                ));
            }
        }
    }
    if bytes.is_empty() {
        return Err("转义序列不能为空".to_string());
    }
    Ok(bytes)
}
