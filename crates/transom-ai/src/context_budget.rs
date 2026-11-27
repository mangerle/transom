//! 上下文窗口 Token 构成拆解与手动压缩摘要辅助。

use transom_core::AiContextBreakdownDto;

use crate::compaction::{entries_since_last_compaction, estimate_tokens};
use crate::memory::MemorySnapshot;
use crate::session::{SessionEntry, SessionEntryKind};

/// 默认估算的工具定义 Schema 占用 Token（11 个核心工具）。
pub const ESTIMATED_TOOLS_SCHEMA_TOKENS: u32 = 2_400;

/// 计算当前会话分支各部分的 Token 构成，供 UI 透明展示。
pub fn compute_context_breakdown(
    preamble: &str,
    memory: &MemorySnapshot,
    entries: &[&SessionEntry],
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
        match &entry.kind {
            SessionEntryKind::Message { text, .. } => {
                history_tokens = history_tokens.saturating_add(estimate_tokens(text));
            }
            SessionEntryKind::Reasoning { text } => {
                history_tokens = history_tokens.saturating_add(estimate_tokens(text));
            }
            SessionEntryKind::Compaction { summary, .. } => {
                history_tokens = history_tokens.saturating_add(estimate_tokens(summary));
            }
            SessionEntryKind::Attachment { content, .. } => {
                history_tokens = history_tokens.saturating_add(estimate_tokens(content));
            }
            SessionEntryKind::ToolRecord {
                summary, output, ..
            } => {
                tool_output_tokens = tool_output_tokens
                    .saturating_add(estimate_tokens(summary))
                    .saturating_add(estimate_tokens(output));
            }
            SessionEntryKind::Header { .. } => {
                history_tokens = history_tokens.saturating_add(16);
            }
        }
    }

    AiContextBreakdownDto {
        system_tokens,
        memory_tokens,
        tools_tokens: ESTIMATED_TOOLS_SCHEMA_TOKENS,
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
                    crate::session::ChatRole::User => "用户",
                    crate::session::ChatRole::Assistant => "助手",
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
    use crate::session::ChatRole;

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
            },
        );
        let e2 = SessionEntry::new(
            Some(e1.id.clone()),
            SessionEntryKind::ToolRecord {
                tool_name: "remote_exec".into(),
                summary: "systemctl status nginx".into(),
                output: "active (running)".into(),
                ok: true,
                approved: Some(true),
                diff_preview: None,
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
            128_000,
        );
        assert!(dto.memory_tokens > 0);
        assert!(dto.history_tokens > 0);
        assert!(dto.tool_output_tokens > 0);
        assert_eq!(dto.tools_tokens, ESTIMATED_TOOLS_SCHEMA_TOKENS);
    }
}
