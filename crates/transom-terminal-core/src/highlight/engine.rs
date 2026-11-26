//! 关键词与模式编译规则集及单行文本扫描引擎。

use super::types::{HighlightKind, HighlightSpan, PRESET_KEYWORDS};
use transom_core::TerminalHighlight;

/// 编译后的规则集：自定义关键词预先转成小写字符向量。
#[derive(Debug, Default, Clone)]
pub struct CompiledHighlight {
    pub(crate) keywords: Vec<Vec<char>>,
}

impl CompiledHighlight {
    /// 按当前配置编译规则集；关闭状态或无自定义关键词时得到空表。
    pub fn compile(config: &TerminalHighlight) -> Self {
        if !config.enabled {
            return Self::default();
        }
        Self {
            keywords: config
                .keywords
                .iter()
                .map(|keyword| keyword.to_lowercase().chars().collect())
                .collect(),
        }
    }

    /// 扫描一行文本，返回按起点升序、互不重叠的高亮区间。
    pub fn scan(&self, text: &str, config: &TerminalHighlight) -> Vec<HighlightSpan> {
        if !config.enabled || text.is_empty() {
            return Vec::new();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut spans = Vec::new();
        let mut index = 0usize;
        while index < chars.len() {
            if let Some((end, kind)) = self.match_at(&chars, index) {
                spans.push(HighlightSpan {
                    start: index,
                    end,
                    kind,
                });
                index = end;
                continue;
            }
            index += 1;
        }
        spans
    }

    /// 在 `index` 处尝试所有规则；返回匹配结束的下一个下标。
    fn match_at(&self, chars: &[char], index: usize) -> Option<(usize, HighlightKind)> {
        if !boundary_ok(chars, index) {
            return None;
        }
        if let Some(found) =
            match_preset(chars, index).or_else(|| match_keyword(chars, index, &self.keywords))
        {
            return Some(found);
        }
        if let Some(end) = match_ipv4(chars, index) {
            return Some((end, HighlightKind::Ipv4));
        }
        match_uuid(chars, index).map(|end| (end, HighlightKind::Uuid))
    }
}

/// 匹配点左侧必须是词边界，否则 `INFORMATION` 会被切成 `INFO`。
fn boundary_ok(chars: &[char], index: usize) -> bool {
    index == 0 || !is_word_char(chars[index - 1])
}

fn boundary_end(chars: &[char], end: usize) -> bool {
    end >= chars.len() || !is_word_char(chars[end])
}

/// 词延续字符：只把 ASCII 字母数字与 `_ . -` 当作"同一个词"。
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

fn match_preset(chars: &[char], index: usize) -> Option<(usize, HighlightKind)> {
    for (bytes, kind) in PRESET_KEYWORDS {
        let len = bytes.len();
        if !fits(chars, index, len) {
            continue;
        }
        let matched = bytes
            .iter()
            .enumerate()
            .all(|(offset, expect)| chars[index + offset] == char::from(*expect));
        if matched && boundary_end(chars, index + len) {
            return Some((index + len, kind));
        }
    }
    None
}

/// 自定义关键词：不区分大小写，同样要求两侧词边界。
fn match_keyword(
    chars: &[char],
    index: usize,
    keywords: &[Vec<char>],
) -> Option<(usize, HighlightKind)> {
    for letters in keywords {
        if !fits(chars, index, letters.len()) {
            continue;
        }
        let matched = letters.iter().enumerate().all(|(offset, expect)| {
            let actual = chars[index + offset];
            actual == *expect || actual.to_ascii_lowercase() == *expect
        });
        if matched && boundary_end(chars, index + letters.len()) {
            return Some((index + letters.len(), HighlightKind::Custom));
        }
    }
    None
}

/// IPv4：四段 1~3 位十进制、每段 ≤255，且两侧不能紧贴数字、点、横线或字母。
fn match_ipv4(chars: &[char], index: usize) -> Option<usize> {
    let mut cursor = index;
    for group in 0..4 {
        cursor = parse_octet(chars, cursor)?;
        if group < 3 {
            if chars.get(cursor) != Some(&'.') {
                return None;
            }
            cursor += 1;
        }
    }
    if !chars[index].is_ascii_digit() {
        return None;
    }
    let next = chars.get(cursor);
    (next.is_none_or(|c| !c.is_ascii_digit() && *c != '.' && !c.is_alphabetic() && *c != '-'))
        .then_some(cursor)
}

fn parse_octet(chars: &[char], start: usize) -> Option<usize> {
    let mut cursor = start;
    let mut value = 0u16;
    let mut digits = 0usize;
    while digits < 3 && chars.get(cursor).is_some_and(char::is_ascii_digit) {
        value = value * 10 + u16::from(chars[cursor].digit_value());
        cursor += 1;
        digits += 1;
    }
    if digits == 0 || value > 255 {
        return None;
    }
    Some(cursor)
}

/// UUID：8-4-4-4-12 的十六进制段，横线分隔，两侧不能紧贴十六进制字符或横线。
fn match_uuid(chars: &[char], index: usize) -> Option<usize> {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut cursor = index;
    for (position, length) in GROUPS.iter().enumerate() {
        if !fits(chars, cursor, *length) {
            return None;
        }
        if !chars[cursor..cursor + length]
            .iter()
            .all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        cursor += length;
        if position < GROUPS.len() - 1 {
            if chars.get(cursor) != Some(&'-') {
                return None;
            }
            cursor += 1;
        }
    }
    let next = chars.get(cursor);
    (next.is_none_or(|c| !c.is_ascii_hexdigit() && *c != '-')).then_some(cursor)
}

fn fits(chars: &[char], index: usize, length: usize) -> bool {
    index + length <= chars.len()
}

trait DigitValue {
    fn digit_value(self) -> u8;
}

impl DigitValue for char {
    fn digit_value(self) -> u8 {
        self as u8 - b'0'
    }
}
