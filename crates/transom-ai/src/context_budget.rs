//! 上下文窗口 Token 构成拆解与手动压缩摘要辅助。

use transom_core::AiContextBreakdownDto;

use crate::compaction::{entries_since_last_compaction, estimate_tokens};
use transom_ai_contracts::memory::MemorySnapshot;
use transom_ai_contracts::session::{SessionEntry, SessionEntryKind};

/// 按单条记录所属桶累加估算 Token（与 [`ContextBudgetTracker`] 共用）。
fn accumulate_entry_tokens(
    entry: &SessionEntry,
    history_tokens: &mut u32,
    tool_output_tokens: &mut u32,
) {
    match &entry.kind {
        SessionEntryKind::Message { text, .. } => {
            *history_tokens = history_tokens.saturating_add(estimate_tokens(text));
        }
        SessionEntryKind::Reasoning { text } => {
            *history_tokens = history_tokens.saturating_add(estimate_tokens(text));
        }
        SessionEntryKind::Compaction { summary, .. } => {
            *history_tokens = estimate_tokens(summary);
            *tool_output_tokens = 0;
        }
        SessionEntryKind::Attachment { content, .. } => {
            *history_tokens = history_tokens.saturating_add(estimate_tokens(content));
        }
        SessionEntryKind::ToolRecord {
            summary, output, ..
        } => {
            *tool_output_tokens = tool_output_tokens
                .saturating_add(estimate_tokens(summary))
                .saturating_add(estimate_tokens(output));
        }
        SessionEntryKind::Header { .. } | SessionEntryKind::AuditNotice { .. } => {
            *history_tokens = history_tokens.saturating_add(16);
        }
    }
}

/// 运行期间增量追踪上下文窗口 Token 构成，避免每轮全量重扫历史记录。
#[derive(Clone, Debug)]
pub struct ContextBudgetTracker {
    dto: AiContextBreakdownDto,
}

impl ContextBudgetTracker {
    /// 基于初始提示词、记忆、已有历史分支与工具 Schema 构造增量追踪器。
    pub fn new(
        preamble: &str,
        memory: &MemorySnapshot,
        entries: &[&SessionEntry],
        tools_tokens: u32,
        context_window: u32,
    ) -> Self {
        Self {
            dto: compute_context_breakdown(preamble, memory, entries, tools_tokens, context_window),
        }
    }

    /// 增量计入一条新追加的会话记录（遇到 `Compaction` 节点时自动重置历史统计）。
    pub fn record_entry(&mut self, entry: &SessionEntry) {
        accumulate_entry_tokens(
            entry,
            &mut self.dto.history_tokens,
            &mut self.dto.tool_output_tokens,
        );
    }

    /// 返回当前上下文构成快照（O(1) 拷贝）。
    pub fn snapshot(&self) -> AiContextBreakdownDto {
        self.dto.clone()
    }
}

/// 计算当前会话分支各部分的 Token 构成，供 UI 透明展示。
pub fn compute_context_breakdown(
    preamble: &str,
    memory: &MemorySnapshot,
    entries: &[&SessionEntry],
    tools_tokens: u32,
    context_window: u32,
) -> AiContextBreakdownDto {
    let memory_tokens = estimate_tokens(&memory.global_markdown)
        .saturating_add(estimate_tokens(&memory.host_markdown));
    let total_preamble_tokens = estimate_tokens(preamble);
    let system_tokens = total_preamble_tokens.saturating_sub(memory_tokens);

    let effective = entries_since_last_compaction(entries);
    let mut history_tokens = 0u32;
    let mut tool_output_tokens = 0u32;

    for entry in effective {
        accumulate_entry_tokens(entry, &mut history_tokens, &mut tool_output_tokens);
    }

    AiContextBreakdownDto {
        system_tokens,
        memory_tokens,
        tools_tokens,
        history_tokens,
        tool_output_tokens,
        context_window,
    }
}

/// 生成本地结构化压缩摘要（支持附带用户指定的保留重点 `focus_instructions`）。
pub fn build_local_compaction_summary(
    entries: &[&SessionEntry],
    focus_instructions: &str,
) -> String {
    let effective = entries_since_last_compaction(entries);
    let mut lines = Vec::new();
    if !focus_instructions.trim().is_empty() {
        lines.push(format!("保留重点：{}", focus_instructions.trim()));
    }
    let mut recent_msgs = Vec::new();
    let mut recent_tools = Vec::new();
    for entry in effective {
        match &entry.kind {
            SessionEntryKind::Message { role, text, .. } => {
                let who = match role {
                    transom_ai_contracts::session::ChatRole::User => "用户",
                    transom_ai_contracts::session::ChatRole::Assistant => "助手",
                };
                let snippet: String = text.chars().take(160).collect();
                recent_msgs.push(format!("- [{who}] {snippet}"));
            }
            SessionEntryKind::ToolRecord {
                tool_name,
                summary,
                ok,
                ..
            } => {
                let mark = if *ok { "成功" } else { "失败" };
                recent_tools.push(format!("- `{tool_name}` ({mark}): {summary}"));
            }
            _ => {}
        }
    }
    if !recent_msgs.is_empty() {
        let start = recent_msgs.len().saturating_sub(6);
        lines.push("关键对话摘要：".to_owned());
        lines.extend(recent_msgs[start..].iter().cloned());
    }
    if !recent_tools.is_empty() {
        let start = recent_tools.len().saturating_sub(8);
        lines.push("关键工具操作：".to_owned());
        lines.extend(recent_tools[start..].iter().cloned());
    }
    if lines.is_empty() {
        "已执行手动上下文压缩。".to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_ai_contracts::session::ChatRole;

    #[test]
    fn breakdown_accounts_for_all_buckets() {
        let e1 = SessionEntry::new(
            None,
            SessionEntryKind::Message {
                role: ChatRole::User,
                text: "请检查 nginx 状态".into(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        let e2 = SessionEntry::new(
            Some(e1.id.clone()),
            SessionEntryKind::ToolRecord {
                tool_name: "remote_exec".into(),
                summary: "systemctl status nginx".into(),
                output: "active (running)".into(),
                ok: true,
                duration_ms: 0,
                approved: Some(true),
                diff_preview: None,
                revert_target_path: None,
                revert_backup_path: None,
            },
        );
        let mem = MemorySnapshot {
            global_markdown: "偏好中文".into(),
            host_markdown: "prod-01 nginx".into(),
        };
        let dto = compute_context_breakdown(
            "系统提示词\n偏好中文\nprod-01 nginx",
            &mem,
            &[&e1, &e2],
            3_600,
            128_000,
        );
        assert!(dto.memory_tokens > 0);
        assert!(dto.history_tokens > 0);
        assert!(dto.tool_output_tokens > 0);
        assert_eq!(dto.tools_tokens, 3_600);
    }

    #[test]
    fn incremental_tracker_matches_full_breakdown() {
        let e1 = SessionEntry::new(
            None,
            SessionEntryKind::Message {
                role: ChatRole::User,
                text: "请检查 nginx 状态".into(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        let e2 = SessionEntry::new(
            Some(e1.id.clone()),
            SessionEntryKind::ToolRecord {
                tool_name: "remote_exec".into(),
                summary: "systemctl status nginx".into(),
                output: "active (running)".into(),
                ok: true,
                duration_ms: 0,
                approved: Some(true),
                diff_preview: None,
                revert_target_path: None,
                revert_backup_path: None,
            },
        );
        let mem = MemorySnapshot {
            global_markdown: "偏好中文".into(),
            host_markdown: "prod-01 nginx".into(),
        };
        let preamble = "系统提示词\n偏好中文\nprod-01 nginx";
        let mut tracker = ContextBudgetTracker::new(preamble, &mem, &[&e1], 3_600, 128_000);
        tracker.record_entry(&e2);
        let full = compute_context_breakdown(preamble, &mem, &[&e1, &e2], 3_600, 128_000);
        assert_eq!(tracker.snapshot(), full);
    }
}
