//! 终端关键词高亮配置模型。
//!
//! 设计原理（Why）：
//! - 本模块只保留纯粹的配置数据契约（开关、自定义关键词列表、清洗与解析逻辑）；
//! - 编译引擎与扫描算法下沉至 `transom-terminal`，避免核心契约库承担具体的字符扫描与正则处理。

use serde::{Deserialize, Serialize};

/// 自定义关键词数量上限：每条都要在字符扫描时匹配，超过这个数量应换用专用词典树。
pub const MAX_KEYWORDS: usize = 32;

/// 单条自定义关键词的长度上限。
pub const MAX_KEYWORD_CHARS: usize = 40;

/// 分隔符集合：半角/全角逗号、顿号、分号与空白均可分隔关键词。
pub const KEYWORD_SEPARATORS: [char; 5] = [',', '，', '、', ';', '；'];

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
    /// 清洗用户输入：去除首尾空白、过滤空项、忽略大小写去重、限制长度与条数。
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

    /// 判断是否有已启用的自定义关键词。
    pub fn is_empty_keywords(&self) -> bool {
        self.keywords.is_empty()
    }

    /// 将设置界面中输入的关键词原始文本切分为条目。
    pub fn parse_keywords(raw: &str) -> Vec<String> {
        raw.split(|c: char| c.is_whitespace() || KEYWORD_SEPARATORS.contains(&c))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// 渲染回输入框展示文本。
    pub fn keywords_joined(&self) -> String {
        self.keywords.join(", ")
    }
}
