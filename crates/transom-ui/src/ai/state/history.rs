//! AI 会话历史到消息流的还原逻辑。

use transom_ai_contracts::{ChatRole, SessionEntry, SessionEntryKind};

use super::{AiPanelState, UiChatItem};
use crate::ai::markdown_blocks::StreamingMarkdownCache;

impl AiPanelState {
    /// 把从 JSONL 加载的树状会话主分支还原为 UI 消息列表，并恢复该会话的推理指标。
    pub(crate) fn load_entries(&mut self, entries: Vec<SessionEntry>) -> Vec<UiChatItem> {
        self.conversation.metrics.restore_from_entries(&entries);
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry.kind {
                SessionEntryKind::Message {
                    role,
                    text,
                    images,
                    model,
                    tokens_per_sec,
                    ..
                } => match role {
                    ChatRole::User => {
                        let parsed_ts = chrono::DateTime::parse_from_rfc3339(&entry.timestamp)
                            .ok()
                            .map(|dt| dt.with_timezone(&chrono::Utc));
                        let ui_images = images
                            .into_iter()
                            .filter(|raw| !raw.trim().is_empty())
                            .map(|raw| {
                                let base64 =
                                    crate::app::ai::composer::image::strip_image_data_url_prefix(
                                        &raw,
                                    )
                                    .to_owned();
                                let thumb =
                                    crate::app::ai::composer::image::decode_base64_render_image(
                                        &base64, 320,
                                    );
                                super::UiChatImage { base64, thumb }
                            })
                            .collect();
                        items.push(UiChatItem::UserText {
                            id: entry.id,
                            text,
                            images: ui_images,
                            timestamp: parsed_ts,
                        });
                    }
                    ChatRole::Assistant => {
                        let cached_segments = Some(StreamingMarkdownCache::from_complete(&text));
                        items.push(UiChatItem::AssistantText {
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
                    items.push(UiChatItem::Reasoning {
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
                    items.push(UiChatItem::ToolCard {
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
                    items.push(UiChatItem::AuditNotice {
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
        items
    }
}
