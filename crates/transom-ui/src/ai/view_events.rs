//! AI 面板运行事件分发与当前对话路由。

use gpui_kit::*;
use transom_core::AiAppEvent;

use super::view::AiAssistantView;

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
            } if self.state.active_conversation_id.as_deref() == Some(conversation_id.as_str()) => {
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
            } if self.state.active_conversation_id.as_deref() == Some(conversation_id.as_str()) => {
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
            } => self.state.active_conversation_id.as_deref() == Some(conversation_id.as_str()),
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
        if let Some(settings) = outcome.settings_snapshot {
            if let Err(err) = self.config_form.apply_settings(
                &settings,
                self.ai_store.as_ref(),
                self.secret_store.as_ref(),
                window,
                cx,
            ) {
                self.state.status_error =
                    Some(rust_i18n::t!("ai.load_config_failed", error = err).to_string());
            }
            self.refresh_skills_list();
            self.probe_active_acp_agent(cx);
        }
        if should_scroll_bottom && !self.auto_scroll_locked {
            self.msg_scroll_handle.scroll_to_bottom();
        }
        if models_discovered {
            self.sync_picker_from_discovered(cx);
        }
        if let Some(cid) = reload_conv {
            self.select_conversation(&cid);
        }
        if refresh_index {
            let store = self.ai_store.clone();
            let bg = cx.background_executor().clone();
            self._refresh_index_task = Some(cx.spawn(async move |this, cx| {
                let conversations = bg
                    .spawn(async move { store.list_all_conversations() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    match conversations {
                        Ok(conversations) => this.state.conversations = conversations,
                        Err(err) => {
                            this.state.status_error = Some(
                                rust_i18n::t!("ai.load_local_data_failed", error = err.to_string())
                                    .to_string(),
                            );
                        }
                    }
                    cx.notify();
                });
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
}
