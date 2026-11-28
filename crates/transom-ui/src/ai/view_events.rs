//! AI 面板运行事件分发与当前对话路由。

use gpui_kit::*;
use transom_core::AiAppEvent;

use super::view::AiAssistantView;
use super::work_group::{MessageBlockCache, group_chat_items};
use crate::app::spawn_blocking_local;

impl AiAssistantView {
    /// 处理控制平面推送的 AI 事件。
    pub(crate) fn handle_app_event(
        &mut self,
        event: AiAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.accepts_event(&event) {
            return;
        }
        let refresh_index = matches!(
            event,
            AiAppEvent::RunStarted { .. } | AiAppEvent::RunFinished { .. }
        );
        let finished_cancelled = match &event {
            AiAppEvent::RunFinished {
                cancelled,
                conversation_id,
                ..
            } if self.state.conversation.active_conversation_id.as_deref()
                == Some(conversation_id.as_str()) =>
            {
                Some(*cancelled)
            }
            _ => None,
        };
        let reload_conv = match &event {
            AiAppEvent::ConversationRolledBack {
                conversation_id, ..
            } => Some(conversation_id.clone()),
            _ => None,
        };
        let subagent_wake = match &event {
            AiAppEvent::SubagentCompleted {
                conversation_id,
                subagent_id,
                title,
                ok,
                summary,
            } if self.state.conversation.active_conversation_id.as_deref()
                == Some(conversation_id.as_str()) =>
            {
                let status = if *ok {
                    rust_i18n::t!("ai.subagent_done_ok").to_string()
                } else {
                    rust_i18n::t!("ai.subagent_done_err").to_string()
                };
                let id_note = if subagent_id.is_empty() {
                    String::new()
                } else {
                    format!(" (subagent_id: {subagent_id})")
                };
                Some(
                    rust_i18n::t!(
                        "ai.subagent_wake_notice",
                        title = title.as_str(),
                        id_note = id_note.as_str(),
                        status = status.as_str(),
                        summary = summary.as_str()
                    )
                    .to_string(),
                )
            }
            _ => None,
        };
        let models_discovered = matches!(event, AiAppEvent::ModelsDiscovered { .. });
        let should_scroll_bottom = match &event {
            AiAppEvent::RunFinished {
                conversation_id, ..
            } => {
                self.state.conversation.active_conversation_id.as_deref()
                    == Some(conversation_id.as_str())
            }
            AiAppEvent::RunStarted { .. }
            | AiAppEvent::TextDelta { .. }
            | AiAppEvent::ReasoningDelta { .. }
            | AiAppEvent::TurnEnd { .. }
            | AiAppEvent::ToolCard(_) => true,
            _ => false,
        };
        if matches!(event, AiAppEvent::RunStarted { .. }) {
            self.auto_scroll_locked = false;
            self.active_nav_dot = None;
        }
        let outcome = self.state.apply_app_event(event);
        self.refresh_message_block_cache();
        if let Some(settings) = outcome.settings_snapshot {
            if let Err(err) = self.config_form.apply_settings(&settings, window, cx) {
                self.state.view.status_error =
                    Some(rust_i18n::t!("ai.load_config_failed", error = err).to_string());
            }
            self.refresh_skills_list(cx);
            self.probe_active_acp_agent(cx);
        }
        if should_scroll_bottom && !self.auto_scroll_locked {
            self.msg_scroll_handle.scroll_to_bottom();
        }
        if models_discovered {
            self.sync_picker_from_discovered(cx);
        }
        if let Some(cid) = reload_conv {
            self.select_conversation(&cid, cx);
        }
        if refresh_index {
            let store = self.ai_store.clone();
            self._refresh_index_task = Some(cx.spawn(async move |this, cx| {
                let conversations =
                    spawn_blocking_local(move || store.list_all_conversations()).await;
                if this
                    .update(cx, |this, cx| {
                        match conversations {
                            Some(Ok(conversations)) => {
                                this.state.conversation.conversations = conversations
                            }
                            Some(Err(err)) => {
                                this.state.view.status_error = Some(
                                    rust_i18n::t!(
                                        "ai.load_local_data_failed",
                                        error = err.to_string()
                                    )
                                    .to_string(),
                                );
                            }
                            None => {
                                this.state.view.status_error =
                                    Some(rust_i18n::t!("ai.background_task_failed").to_string());
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    log::debug!("AI 会话索引刷新完成时视图已关闭，结果被丢弃");
                }
            }));
        }
        if let Some(cancelled) = finished_cancelled {
            self.maybe_drain_queue_after_finish(cancelled, cx);
        }
        if let Some(wake_prompt) = subagent_wake {
            if !self.state.has_active_run() {
                self.send_prompt_text(wake_prompt, cx);
            } else {
                self.state
                    .run
                    .queued_messages
                    .push(super::queue_card::QueuedPrompt {
                        id: uuid::Uuid::new_v4().to_string(),
                        prompt: wake_prompt,
                        attachments: Vec::new(),
                    });
            }
        }
        cx.notify();
    }

    /// 在消息结构或运行态变化后重建分组区间缓存。
    pub(super) fn refresh_message_block_cache(&mut self) {
        let is_running = self.state.active_run_is_current();
        let item_count = self.state.conversation.items.len();
        let group_revision = self.state.conversation.group_revision;
        let cache_is_valid = self.message_block_cache.as_ref().is_some_and(|cache| {
            cache.item_count == item_count
                && cache.is_running == is_running
                && cache.group_revision == group_revision
        });
        if cache_is_valid {
            return;
        }
        let blocks = group_chat_items(&self.state.conversation.items, is_running);
        self.message_block_cache = Some(MessageBlockCache {
            item_count,
            is_running,
            group_revision,
            blocks,
        });
    }
}
