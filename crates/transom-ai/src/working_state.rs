//! 会话工作状态提取、上下文超限（413）识别与激进降级压缩。
//!
//! 设计原理（Why）：
//! - 即使在发送前按字符估算了 Token，不同模型的分词器仍可能触发 `413` 或 `context_length_exceeded`；
//! - 长会话裁剪旧消息后，模型容易遗忘初始目标与已排查结论；
//! - 本模块从现有分支提取轻量的 [`SessionWorkingState`]，在激进压缩后将其重注入系统提示词，确保降级重试不失忆。

use crate::compaction::truncate_tool_output;
use crate::session::{ChatRole, SessionEntry, SessionEntryKind};

/// 供压缩重注入的轻量会话工作状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionWorkingState {
    pub user_goal: String,
    pub confirmed_facts: Vec<String>,
    pub edited_files: Vec<String>,
    pub blockers: Vec<String>,
    pub recent_output_handles: Vec<String>,
}

impl SessionWorkingState {
    /// 从历史分支与当前提问提炼会话工作状态。
    pub fn from_history(entries: &[SessionEntry], current_prompt: &str) -> Self {
        let mut state = Self::default();
        for entry in entries {
            match &entry.kind {
                SessionEntryKind::Message { role, text, .. } if *role == ChatRole::User => {
                    if state.user_goal.is_empty() && !text.trim().is_empty() {
                        state.user_goal = clip_one_line(text, 180);
                    }
                }
                SessionEntryKind::ToolRecord {
                    tool_name,
                    summary,
                    output,
                    ok,
                    ..
                } => {
                    if *ok {
                        if tool_name == "remote_fs"
                            && (summary.starts_with("edit ") || summary.starts_with("mkdir "))
                        {
                            push_unique(&mut state.edited_files, summary.clone(), 12);
                        } else if tool_name == "propose_host_memory" {
                            push_unique(&mut state.confirmed_facts, summary.clone(), 10);
                        }
                    } else {
                        push_unique(
                            &mut state.blockers,
                            format!(
                                "{tool_name}({}): {}",
                                clip_one_line(summary, 60),
                                clip_one_line(output, 80)
                            ),
                            8,
                        );
                    }
                    extract_output_handles(output, &mut state.recent_output_handles);
                }
                _ => {}
            }
        }
        if state.user_goal.is_empty() && !current_prompt.trim().is_empty() {
            state.user_goal = clip_one_line(current_prompt, 180);
        }
        state
    }

    /// 渲染为可注入系统提示词的紧凑 Markdown 块。
    pub fn render_markdown(&self) -> String {
        let mut lines = Vec::new();
        if !self.user_goal.is_empty() {
            lines.push(format!("- 初始目标: {}", self.user_goal));
        }
        if !self.confirmed_facts.is_empty() {
            lines.push(format!("- 已确认结论: {}", self.confirmed_facts.join("；")));
        }
        if !self.edited_files.is_empty() {
            lines.push(format!("- 已修改路径: {}", self.edited_files.join("，")));
        }
        if !self.blockers.is_empty() {
            lines.push(format!("- 最近异常/阻塞: {}", self.blockers.join("；")));
        }
        if !self.recent_output_handles.is_empty() {
            lines.push(format!(
                "- 可读取的完整输出句柄: {}",
                self.recent_output_handles.join("，")
            ));
        }
        lines.join("\n")
    }
}

/// 判断模型调用错误原因是否属于请求体过大或上下文超限（413 / Context Overflow）。
pub fn is_context_overflow_error(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    const MARKERS: [&str; 9] = [
        "413",
        "context_length_exceeded",
        "maximum context length",
        "prompt is too long",
        "too many tokens",
        "request entity too large",
        "payload too large",
        "input is too long",
        "token limit",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

/// 对历史分支执行激进压缩：在头部插入合成 Compaction 摘要，仅保留最近 `keep_recent` 条记录并收紧旧工具输出。
pub fn aggressive_compact_history(
    entries: &[SessionEntry],
    state: &SessionWorkingState,
    keep_recent: usize,
    max_tool_chars: usize,
) -> Vec<SessionEntry> {
    let tail_start = entries.len().saturating_sub(keep_recent.max(1));
    let mut compacted = Vec::with_capacity(entries.len().saturating_sub(tail_start) + 1);

    let summary_text = {
        let rendered = state.render_markdown();
        if rendered.is_empty() {
            "早期历史已压缩，请基于最近上下文继续完成任务。".to_owned()
        } else {
            format!("早期历史已压缩，关键工作状态如下:\n{rendered}")
        }
    };
    let first_kept = entries
        .get(tail_start)
        .map(|e| e.id.clone())
        .unwrap_or_default();
    compacted.push(SessionEntry::new(
        None,
        SessionEntryKind::Compaction {
            summary: summary_text,
            first_kept_entry_id: first_kept,
        },
    ));

    for entry in &entries[tail_start..] {
        let mut cloned = entry.clone();
        if let SessionEntryKind::ToolRecord { output, .. } = &mut cloned.kind {
            *output = truncate_tool_output(output, max_tool_chars.max(128));
        } else if let SessionEntryKind::Attachment { content, .. } = &mut cloned.kind {
            *content = truncate_tool_output(content, max_tool_chars.max(128));
        }
        compacted.push(cloned);
    }
    compacted
}

fn push_unique(list: &mut Vec<String>, value: String, cap: usize) {
    let trimmed = value.trim();
    if trimmed.is_empty() || list.iter().any(|existing| existing == trimmed) {
        return;
    }
    list.push(trimmed.to_owned());
    if list.len() > cap {
        list.remove(0);
    }
}

fn clip_one_line(raw: &str, max_chars: usize) -> String {
    let single = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = single.chars().count();
    if count <= max_chars {
        single
    } else {
        let head: String = single.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn extract_output_handles(text: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(pos) = rest.find("output_id=") {
        let after = &rest[pos + "output_id=".len()..];
        let id: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if !id.is_empty() {
            push_unique(out, id, 8);
        }
        rest = after;
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionWorkingState, aggressive_compact_history, is_context_overflow_error};
    use crate::session::{ChatRole, SessionEntry, SessionEntryKind};

    #[test]
    fn detects_context_overflow_variants() {
        assert!(is_context_overflow_error(
            "HTTP 413 Request Entity Too Large"
        ));
        assert!(is_context_overflow_error(
            "error code: context_length_exceeded"
        ));
        assert!(is_context_overflow_error(
            "This model's maximum context length is 128000"
        ));
        assert!(!is_context_overflow_error("connection reset by peer"));
    }

    #[test]
    fn extracts_working_state_and_compacts_history() {
        let e1 = SessionEntry::new(
            None,
            SessionEntryKind::Message {
                role: ChatRole::User,
                text: "排查 nginx 502 错误".to_owned(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        let e2 = SessionEntry::new(
            Some(e1.id.clone()),
            SessionEntryKind::ToolRecord {
                tool_name: "remote_fs".to_owned(),
                summary: "edit /etc/nginx/nginx.conf".to_owned(),
                output: format!("{} output_id=out-1234abcd", "x".repeat(2000)),
                ok: true,
                duration_ms: 0,
                approved: Some(true),
                diff_preview: None,
            },
        );
        let e3 = SessionEntry::new(
            Some(e2.id.clone()),
            SessionEntryKind::ToolRecord {
                tool_name: "remote_exec".to_owned(),
                summary: "nginx -t".to_owned(),
                output: "syntax ok".to_owned(),
                ok: false,
                duration_ms: 0,
                approved: Some(true),
                diff_preview: None,
            },
        );

        let entries = vec![e1, e2, e3];
        let state = SessionWorkingState::from_history(&entries, "");
        assert_eq!(state.user_goal, "排查 nginx 502 错误");
        assert_eq!(state.edited_files, vec!["edit /etc/nginx/nginx.conf"]);
        assert_eq!(state.recent_output_handles, vec!["out-1234abcd"]);
        assert_eq!(state.blockers.len(), 1);

        let compacted = aggressive_compact_history(&entries, &state, 2, 200);
        assert_eq!(compacted.len(), 3);
        assert!(matches!(
            compacted[0].kind,
            SessionEntryKind::Compaction { .. }
        ));
    }
}
