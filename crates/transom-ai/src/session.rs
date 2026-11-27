//! 树状追加写 JSONL 会话模型。
//!
//! 设计原理（Why）：
//! - 借鉴 `pi` 的追加写 JSONL 树状结构：每条记录携带 `id` 与 `parent_id`，仅在文件末尾追加，
//!   避免大对话整文件重写导致的 IO 放大与崩溃损坏；
//! - 原生支持“从某条消息重新生成/分叉”，以及追加 [`SessionEntryKind::Compaction`] 摘要节点
//!   而不破坏用户可见的完整历史。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AiError;
use crate::thinking::ThinkingEffort;

/// 对话的作用域归属。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum SessionScope {
    /// 绑定到指定主机连接。
    Host(String),
    /// 通用/本地问答，不绑定远端主机。
    #[default]
    Global,
}

/// 对话索引条目（存储在 `index.json` 中用于秒开侧栏列表）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub scope: SessionScope,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub effort: Option<ThinkingEffort>,
    pub updated_at: String,
    #[serde(default)]
    pub active_leaf_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

/// 树状会话中的单条记录（对应 JSONL 文件中的一行）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub timestamp: String,
    #[serde(flatten)]
    pub kind: SessionEntryKind,
}

impl SessionEntry {
    /// 构造新条目并生成唯一 ID 与当前时间戳。
    pub fn new(parent_id: Option<String>, kind: SessionEntryKind) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            parent_id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind,
        }
    }
}

/// 单轮助手回复的推理指标持久化记录（写入 JSONL 供历史会话恢复展示）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsageRecord {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub model_duration_ms: u64,
    #[serde(default)]
    pub ttft_ms: Option<u64>,
}

/// 会话条目的具体载荷类型。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entry_type", rename_all = "snake_case")]
pub enum SessionEntryKind {
    /// 会话头信息。
    Header {
        title: String,
        scope: SessionScope,
        #[serde(default)]
        model_id: String,
    },
    /// 用户或助手正文消息。
    Message {
        role: ChatRole,
        text: String,
        #[serde(default)]
        images: Vec<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        tokens_per_sec: Option<f32>,
        #[serde(default)]
        usage: Option<TurnUsageRecord>,
    },
    /// 推理模型的思考块。
    Reasoning { text: String },
    /// 工具调用与执行结果卡片。
    ToolRecord {
        tool_name: String,
        summary: String,
        output: String,
        ok: bool,
        #[serde(default)]
        duration_ms: u64,
        #[serde(default)]
        approved: Option<bool>,
        #[serde(default)]
        diff_preview: Option<String>,
    },
    /// 上下文压缩摘要边界：后续组装发送给模型时从该节点起算。
    Compaction {
        summary: String,
        first_kept_entry_id: String,
    },
    /// 附加的终端命令块或本地参考快照。
    Attachment { label: String, content: String },
    /// 审批决策或权限变更的轻量化审计锚点。
    AuditNotice {
        approved: bool,
        label: String,
        #[serde(default)]
        host_label: String,
    },
}

/// 对话消息角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

/// 内存中的树状会话视图。
#[derive(Clone, Debug, Default)]
pub struct SessionTree {
    entries: Vec<SessionEntry>,
    by_id: HashMap<String, usize>,
    active_leaf_id: Option<String>,
}

impl SessionTree {
    /// 从 JSONL 多行文本解析会话树；忽略空行，遇到损坏行则返回错误。
    pub fn from_jsonl(content: &str) -> Result<Self, AiError> {
        let mut tree = Self::default();
        for (line_no, raw_line) in content.lines().enumerate() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: SessionEntry =
                serde_json::from_str(trimmed).map_err(|err| AiError::StorageFailed {
                    target: format!("会话 JSONL 第 {} 行", line_no + 1),
                    reason: err.to_string(),
                })?;
            tree.push(entry);
        }
        Ok(tree)
    }

    /// 将单条记录序列化为单行 JSON（供追加写入文件末尾）。
    pub fn encode_entry(entry: &SessionEntry) -> Result<String, AiError> {
        serde_json::to_string(entry).map_err(|err| AiError::StorageFailed {
            target: "序列化会话条目".to_owned(),
            reason: err.to_string(),
        })
    }

    /// 追加一条记录并将当前活动叶节点推进至该记录。
    pub fn push(&mut self, entry: SessionEntry) {
        let idx = self.entries.len();
        self.active_leaf_id = Some(entry.id.clone());
        self.by_id.insert(entry.id.clone(), idx);
        self.entries.push(entry);
    }

    /// 切换活动叶节点（用于分支回溯或重试）。
    pub fn set_active_leaf(&mut self, leaf_id: Option<String>) {
        if let Some(ref id) = leaf_id
            && self.by_id.contains_key(id)
        {
            self.active_leaf_id = leaf_id;
        }
    }

    /// 当前活动叶节点 ID。
    pub fn active_leaf_id(&self) -> Option<&str> {
        self.active_leaf_id.as_deref()
    }

    /// 从当前叶节点回溯到根节点，返回按时间正序排列的主分支记录列表。
    pub fn active_branch(&self) -> Vec<&SessionEntry> {
        let mut chain = Vec::new();
        let mut cursor = self.active_leaf_id.as_deref();
        let mut guard = 0usize;
        while let Some(id) = cursor {
            if guard > self.entries.len() {
                break;
            }
            guard += 1;
            let Some(&idx) = self.by_id.get(id) else {
                break;
            };
            let entry = &self.entries[idx];
            chain.push(entry);
            cursor = entry.parent_id.as_deref();
        }
        chain.reverse();
        chain
    }

    /// 返回全部原始条目。
    pub fn all_entries(&self) -> &[SessionEntry] {
        &self.entries
    }
}

/// 从用户输入中提取 `@主机名` 提及（不误匹配 `user@host` 邮箱/登录串，兼容紧邻中文标点），并按可用主机列表大小写不敏感去重返回。
pub fn extract_host_mentions(prompt: &str, available_hosts: &[&str]) -> Vec<String> {
    let mut matched = Vec::new();
    let mut sorted_hosts = available_hosts.to_vec();
    sorted_hosts.sort_by_key(|h| std::cmp::Reverse(h.len()));

    for (idx, ch) in prompt.char_indices() {
        if ch != '@' {
            continue;
        }
        let prev = prompt[..idx].chars().next_back();
        if prev.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            continue;
        }
        let after = &prompt[idx + 1..];
        for &candidate in &sorted_hosts {
            if let Some(prefix) = after.get(..candidate.len())
                && prefix.eq_ignore_ascii_case(candidate)
            {
                let next_ch = after[candidate.len()..].chars().next();
                let boundary_ok =
                    next_ch.is_none_or(|c| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '-'));
                if boundary_ok && !matched.iter().any(|existing| existing == candidate) {
                    matched.push(candidate.to_owned());
                    break;
                }
            }
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::{ChatRole, SessionEntry, SessionEntryKind, SessionScope, SessionTree};

    #[test]
    fn tree_branch_follows_parent_chain() {
        let mut tree = SessionTree::default();
        let header = SessionEntry::new(
            None,
            SessionEntryKind::Header {
                title: "排查 Nginx".to_owned(),
                scope: SessionScope::Host("host-1".to_owned()),
                model_id: "m1".to_owned(),
            },
        );
        let root_id = header.id.clone();
        tree.push(header);

        let u1 = SessionEntry::new(
            Some(root_id.clone()),
            SessionEntryKind::Message {
                role: ChatRole::User,
                text: "看看状态".to_owned(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        let u1_id = u1.id.clone();
        tree.push(u1);

        let a1 = SessionEntry::new(
            Some(u1_id.clone()),
            SessionEntryKind::Message {
                role: ChatRole::Assistant,
                text: "旧回答".to_owned(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        tree.push(a1);

        // 从 u1 分叉重试
        let a2 = SessionEntry::new(
            Some(u1_id),
            SessionEntryKind::Message {
                role: ChatRole::Assistant,
                text: "新回答".to_owned(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        tree.push(a2);

        let branch = tree.active_branch();
        assert_eq!(branch.len(), 3);
        if let SessionEntryKind::Message { text, .. } = &branch[2].kind {
            assert_eq!(text, "新回答");
        } else {
            panic!("最后一个节点应为 Assistant 消息");
        }
    }

    #[test]
    fn jsonl_roundtrip_preserves_entries() {
        let entry = SessionEntry::new(
            None,
            SessionEntryKind::Reasoning {
                text: "先检查端口".to_owned(),
            },
        );
        let line = SessionTree::encode_entry(&entry).expect("序列化失败");
        let parsed = SessionTree::from_jsonl(&line).expect("反序列化失败");
        assert_eq!(parsed.all_entries().len(), 1);
    }

    #[test]
    fn extracts_host_mentions_without_email_false_positives() {
        let hosts = ["web-01", "db-01", "cache-02"];
        let got = super::extract_host_mentions(
            "让 ops@10.0.0.1 去检查 @web-01，顺便对比 @DB-01 和 @web-01",
            &hosts,
        );
        assert_eq!(got, vec!["web-01".to_owned(), "db-01".to_owned()]);
    }
}
