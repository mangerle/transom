//! AI 面板运行事件分发与当前对话路由。

use gpui_kit::*;
use transom_core::AiAppEvent;

use super::message_stream::MessageStreamSnapshot;
use super::view::AiAssistantView;
use crate::app::spawn_blocking_local;

impl AiAssistantView {
    /// 将当前运行态元信息投影同步到独立的 [`MessageStreamView`] 实体（零消息列表克隆）。
    pub(super) fn sync_message_stream(
        &mut self,
        reset_scroll_lock: bool,
        scroll_to_bottom: bool,
        cx: &mut Context<Self>,
    ) {
        let active_acp = self.active_acp_agent().map(|agent| (agent.id, agent.name));
        let snap = MessageStreamSnapshot {
            is_running: self.state.active_run_is_current(),
            run_started_at: self.state.current_run_started_at(),
            active_focus: self.state.run.active_focus.clone(),
            status_error: self.state.view.status_error.clone(),
            copied_key: self.copied_key.clone(),
            has_active_terminal: self.state.conversation.active_session.is_some(),
            active_acp,
            managed_tasks: self.state.run.managed_tasks.clone(),
        };
        self.message_stream.update(cx, |stream, cx| {
            stream.sync_snapshot(snap, reset_scroll_lock, scroll_to_bottom, cx);
        });
    }

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
        // 高频流式增量仅局部刷新 MessageStreamView 实体，彻底隔离顶栏、抽屉与输入区重绘。
        match &event {
            AiAppEvent::TextDelta { delta, .. } => {
                self.message_stream.update(cx, |stream, cx| {
                    stream.apply_streaming_delta(false, delta, cx);
                });
                return;
            }
            AiAppEvent::ReasoningDelta { delta, .. } => {
                self.message_stream.update(cx, |stream, cx| {
                    stream.apply_streaming_delta(true, delta, cx);
                });
                return;
            }
            _ => {}
        }

        let is_run_started = matches!(event, AiAppEvent::RunStarted { .. });
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
            | AiAppEvent::TurnEnd { .. }
            | AiAppEvent::ToolCard(_) => true,
            _ => false,
        };
        let active_model_name = self.state.active_model_name();
        let is_current_conv = matches!(
            &event,
            AiAppEvent::RunFinished { conversation_id, .. }
                if self.state.conversation.active_conversation_id.as_deref()
                    == Some(conversation_id.as_str())
        );
        self.message_stream.update(cx, |stream, cx| {
            stream.apply_app_event(&event, active_model_name, is_current_conv, cx);
        });
        let outcome = self.state.apply_app_event(event);
        if let Some(settings) = outcome.settings_snapshot {
            if let Err(err) = self.config_form.apply_settings(&settings, window, cx) {
                self.state.view.status_error =
                    Some(rust_i18n::t!("ai.load_config_failed", error = err).to_string());
            }
            let disabled = settings.disabled_skills.clone();
            self.skills_drawer.update(cx, |drawer, cx| {
                drawer.set_disabled_and_refresh(disabled, cx)
            });
            self.probe_active_acp_agent(cx);
        }
        self.sync_message_stream(is_run_started, should_scroll_bottom, cx);
        if models_discovered {
            self.sync_picker_from_discovered(cx);
        }
        if let Some(cid) = reload_conv {
            self.select_conversation(&cid, cx);
        }
        if refresh_index {
            let store = self.ai_store.clone();
            self.tasks.refresh_index = Some(cx.spawn(async move |this, cx| {
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
}
