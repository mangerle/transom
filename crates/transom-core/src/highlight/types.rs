//! 关键词高亮配置模型与色相契约。

use serde::{Deserialize, Serialize};

/// 内建的日志级别关键词及其色相归属。
pub(crate) const PRESET_KEYWORDS: [(&[u8], HighlightKind); 6] = [
    (b"ERROR", HighlightKind::Error),
    (b"FATAL", HighlightKind::Error),
    (b"WARN", HighlightKind::Warning),
    (b"WARNING", HighlightKind::Warning),
    (b"INFO", HighlightKind::Info),
    (b"DEBUG", HighlightKind::Debug),
];

/// 自定义关键词数量上限：每条都要在每个字符位置上试一次，超过这个量就该换成词典树。
pub(crate) const MAX_KEYWORDS: usize = 32;
/// 单条自定义关键词的长度上限。
pub(crate) const MAX_KEYWORD_CHARS: usize = 40;

/// 分隔符集合：半角/全角逗号、顿号、分号与任意空白都能分隔一个关键词。
pub(crate) const KEYWORD_SEPARATORS: [char; 5] = [',', '，', '、', ';', '；'];

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

/// 关键词高亮配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHighlight {
    /// 总开关。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 自定义关键词，匹配时不区分大小写；保存前经 [`TerminalHighlight::sanitized`] 清洗。
    #[serde(default)]
    pub keywords: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

impl Default for TerminalHighlight {
    fn default() -> Self {
        Self {
            enabled: true,
            keywords: Vec::new(),
        }
    }
}

impl TerminalHighlight {
    /// 清洗用户输入：去首尾空白、丢空项、不区分大小写去重、限长限条数。
    pub fn sanitized(self) -> Self {
        let mut keywords: Vec<String> = Vec::with_capacity(self.keywords.len().min(MAX_KEYWORDS));
        for keyword in self.keywords {
            let trimmed = keyword.trim();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_KEYWORD_CHARS {
                continue;
            }
            let lowered = trimmed.to_lowercase();
            if keywords
                .iter()
                .any(|existing| existing.to_lowercase() == lowered)
            {
                continue;
            }
            if keywords.len() >= MAX_KEYWORDS {
                break;
            }
            keywords.push(trimmed.to_string());
        }
        Self {
            enabled: self.enabled,
            keywords,
        }
    }

    pub fn is_empty_keywords(&self) -> bool {
        self.keywords.is_empty()
    }

    /// 把设置里的"关键词列表"文本切成条目。
    pub fn parse_keywords(raw: &str) -> Vec<String> {
        raw.split(|c: char| c.is_whitespace() || KEYWORD_SEPARATORS.contains(&c))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// 渲染回输入框文本（保存后回填用）。
    pub fn keywords_joined(&self) -> String {
        self.keywords.join(", ")
    }
}
