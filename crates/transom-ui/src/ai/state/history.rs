//! AI 会话历史到消息流的还原逻辑。

use std::sync::Arc;

use transom_ai::{ChatRole, SessionEntry, SessionEntryKind};

use super::{AiPanelState, UiChatItem};
use crate::ai::markdown_blocks::parse_markdown_segments;

impl AiPanelState {
    /// 把从 JSONL 加载的树状会话主分支还原为 UI 消息列表，并恢复该会话的推理指标。
    pub(crate) fn load_entries(&mut self, entries: Vec<SessionEntry>) {
        self.conversation.items.clear();
        self.conversation.metrics.restore_from_entries(&entries);
        for entry in entries {
            match entry.kind {
                SessionEntryKind::Message {
                    role,
                    text,
                    model,
                    tokens_per_sec,
                    ..
                } => match role {
                    ChatRole::User => {
                        self.conversation.items.push(UiChatItem::UserText {
                            id: entry.id,
                            text,
                            timestamp: Some(entry.timestamp),
                        });
                    }
                    ChatRole::Assistant => {
                        let cached_segments = Some(Arc::new(parse_markdown_segments(&text)));
                        self.conversation.items.push(UiChatItem::AssistantText {
                            id: entry.id,
                            text,
                            model,
                            tokens_per_sec,
                            streaming: false,
                            cached_segments,
                        });
                    }
                },
                SessionEntryKind::Reasoning { text } => {
                    self.conversation.items.push(UiChatItem::Reasoning {
                        id: entry.id,
                        text,
                        streaming: false,
                    });
                }
                SessionEntryKind::ToolRecord {
                    tool_name,
                    summary,
                    output,
                    ok,
                    duration_ms,
                    diff_preview,
                    revert_target_path,
                    revert_backup_path,
                    ..
                } => {
                    self.conversation.items.push(UiChatItem::ToolCard {
                        id: entry.id,
                        call_id: String::new(),
                        running: false,
                        tool_name,
                        host_label: self
                            .conversation
                            .active_session
                            .as_ref()
                            .map(|session| session.host_name.clone())
                            .unwrap_or_else(|| rust_i18n::t!("ai.general_label").to_string()),
                        risk_label: String::new(),
                        summary,
                        output,
                        ok,
                        duration_ms,
                        diff_preview,
                        revert_target_path,
                        revert_backup_path,
                    });
                }
                SessionEntryKind::AuditNotice {
                    approved,
                    label,
                    host_label,
                } => {
                    self.conversation.items.push(UiChatItem::AuditNotice {
                        id: entry.id,
                        approved,
                        label,
                        host_label,
                    });
                }
                SessionEntryKind::Header { .. }
                | SessionEntryKind::Compaction { .. }
                | SessionEntryKind::Attachment { .. } => {}
            }
        }
    }
}
