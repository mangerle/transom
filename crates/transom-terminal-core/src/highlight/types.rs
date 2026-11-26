//! 终端关键词高亮类别与区间契约。

/// 内建的日志级别关键词及其色相归属。
pub(crate) const PRESET_KEYWORDS: [(&[u8], HighlightKind); 6] = [
    (b"ERROR", HighlightKind::Error),
    (b"FATAL", HighlightKind::Error),
    (b"WARN", HighlightKind::Warning),
    (b"WARNING", HighlightKind::Warning),
    (b"INFO", HighlightKind::Info),
    (b"DEBUG", HighlightKind::Debug),
];

/// 高亮类别。颜色映射到会话调色板的 ANSI 槽位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// 错误级别（ERROR/FATAL）。
    Error,
    /// 告警级别（WARN/WARNING）。
    Warning,
    /// 信息级别（INFO）。
    Info,
    /// 调试级别（DEBUG）。
    Debug,
    /// IPv4 地址。
    Ipv4,
    /// UUID。
    Uuid,
    /// 用户自定义关键词。
    Custom,
}

impl HighlightKind {
    /// 取会话调色板里对应 ANSI 槽位的颜色（0..16）。
    pub fn ansi_index(self) -> usize {
        match self {
            HighlightKind::Error => 9,
            HighlightKind::Warning => 11,
            HighlightKind::Info => 12,
            HighlightKind::Debug => 7,
            HighlightKind::Ipv4 => 14,
            HighlightKind::Uuid => 13,
            HighlightKind::Custom => 10,
        }
    }
}

/// 一行内需要上色的区间，下标是**字符序号**（不是字节），区间为 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}
