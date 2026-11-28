//! AI 助手面板会话与消息发送交互动作。

use gpui_kit::*;
use transom_ai_contracts::extract_host_mentions;
use transom_core::{
    AiCommand, AiHostTargetDto, AiPromptRequest, AppCommand, HostProtection, TabId,
};

use super::AiPanelEvent;
use super::attachments::pack_attachments_for_request;
use super::state::{AiActivePopup, AiDrawerTab, UiChatItem};
use super::view::AiAssistantView;
use crate::app::spawn_blocking_local;

impl AiAssistantView {
    /// 向控制平面提交 AI 命令；通道满或关闭时在当前面板展示错误并返回失败。
    pub(super) fn send_ai_command(&mut self, command: AiCommand, cx: &mut Context<Self>) -> bool {
        match self.command_tx.try_send(AppCommand::Ai(Box::new(command))) {
            Ok(()) => true,
            Err(err) => {
                self.state.view.status_error = Some(
                    rust_i18n::t!("ai.command_send_failed", error = err.to_string()).to_string(),
                );
                cx.notify();
                false
            }
        }
    }

    pub(super) fn select_conversation(&mut self, conv_id: &str, cx: &mut Context<Self>) {
        self.active_nav_dot = None;
        self.editing_title = false;
        self.state.conversation.load_seq = self.state.conversation.load_seq.wrapping_add(1);
        let load_seq = self.state.conversation.load_seq;
        self.state.conversation.active_conversation_id = Some(conv_id.to_owned());
        self.state.conversation.active_conversation_title = self
            .state
            .conversation
            .conversations
            .iter()
            .find(|c| c.id == conv_id)
            .map(|c| c.title.clone());
        self.state.conversation.metrics.reset_session();
        if !self.state.active_run_is_current() {
            self.state.run.active_plan = None;
            self.state.run.active_focus = None;
            self.state.view.status_error = None;
        }
        let store = self.ai_store.clone();
        let conversation_id = conv_id.to_owned();
        let display_conversation_id = conv_id.to_owned();
        self._conversation_load_task = Some(cx.spawn(async move |this, cx| {
            let loaded =
                spawn_blocking_local(move || store.load_session_tree(&conversation_id)).await;
            let loaded = loaded.unwrap_or_else(|| {
                Err(transom_ai_contracts::AiError::StorageFailed {
                    target: "ai_session_load".to_owned(),
                    reason: rust_i18n::t!("ai.background_task_failed").to_string(),
                })
            });
            if this
                .update(cx, |this, cx| {
                    if this.state.conversation.load_seq != load_seq {
                        return;
                    }
                    match loaded {
                        Ok(tree) => {
                            let branch: Vec<_> =
                                tree.active_branch().into_iter().cloned().collect();
                            this.state.load_entries(branch);
                            this.refresh_message_block_cache();
                            this.state.view.status_error = this
                                .state
                                .run
                                .background_run_errors
                                .remove(&display_conversation_id);
                        }
                        Err(err) => {
                            this.state.conversation.items.clear();
                            this.refresh_message_block_cache();
                            this.state
                                .run
                                .background_run_errors
                                .remove(&display_conversation_id);
                            this.state.view.status_error = Some(
                                rust_i18n::t!(
                                    "ai.load_conversation_failed",
                                    error = err.to_string()
                                )
                                .to_string(),
                            );
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("AI 会话读取完成时视图已关闭，结果被丢弃");
            }
        }));
    }

    pub(super) fn delete_conversation(&mut self, conv_id: &str, cx: &mut Context<Self>) {
        if !self.send_ai_command(
            AiCommand::DeleteConversation {
                conversation_id: conv_id.to_owned(),
            },
            cx,
        ) {
            return;
        }
        self.state
            .conversation
            .conversations
            .retain(|c| c.id != conv_id);
        if self.state.conversation.active_conversation_id.as_deref() == Some(conv_id) {
            self.clear_conversation(cx);
        }
        cx.notify();
    }

    pub(super) fn set_conversation_archived(
        &mut self,
        conv_id: &str,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.send_ai_command(
            AiCommand::SetConversationArchived {
                conversation_id: conv_id.to_owned(),
                archived,
            },
            cx,
        ) {
            return;
        }
        if let Some(conv) = self
            .state
            .conversation
            .conversations
            .iter_mut()
            .find(|c| c.id == conv_id)
        {
            conv.archived = archived;
        }
        cx.notify();
    }

    pub(super) fn start_editing_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.conversation.active_conversation_id.is_none() {
            return;
        }
        let cur = self
            .state
            .conversation
            .active_conversation_title
            .clone()
            .unwrap_or_default();
        self.editing_title = true;
        self.title_input.update(cx, |input, cx| {
            input.set_value(cur, window, cx);
            input.focus_handle(cx).focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn commit_title_edit(&mut self, cx: &mut Context<Self>) {
        if !self.editing_title {
            return;
        }
        self.editing_title = false;
        let next = self.title_input.read(cx).value().trim().to_owned();
        if let Some(cid) = self.state.conversation.active_conversation_id.clone()
            && !next.is_empty()
        {
            if !self.send_ai_command(
                AiCommand::RenameConversation {
                    conversation_id: cid.clone(),
                    title: next.clone(),
                },
                cx,
            ) {
                return;
            }
            self.state.conversation.active_conversation_title = Some(next.clone());
            if let Some(conv) = self
                .state
                .conversation
                .conversations
                .iter_mut()
                .find(|c| c.id == cid)
            {
                conv.title = next.clone();
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_extra_scope_tab(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if !self.state.conversation.extra_scope_tabs.remove(&tab_id) {
            self.state.conversation.extra_scope_tabs.insert(tab_id);
        }
        cx.notify();
    }

    pub(super) fn send_prompt_text(&mut self, raw_prompt: String, cx: &mut Context<Self>) {
        let mut prompt = raw_prompt.trim().to_owned();
        if self.state.has_active_run() {
            if !self.state.active_run_is_current() {
                self.state.view.status_error =
                    Some(rust_i18n::t!("ai.background_run_active").to_string());
                cx.notify();
                return;
            }
            if !prompt.is_empty() || !self.state.composer.pending_attachments.is_empty() {
                self.handle_running_message(prompt, cx);
            }
            return;
        }
        self.state.run.queue_paused_by_cancel = false;
        self.auto_scroll_locked = false;
        self.active_nav_dot = None;
        self.msg_scroll_handle.scroll_to_bottom();
        let packed = pack_attachments_for_request(&self.state.composer.pending_attachments);
        let images_base64 = packed.images;
        let attachment_text = packed.text;
        if prompt.is_empty() {
            if images_base64.is_empty() && attachment_text.is_none() {
                return;
            }
            prompt = rust_i18n::t!("ai.analyze_attachments_prompt").to_string();
        }
        self.state.composer.pending_attachments.clear();
        let primary_tab = self
            .state
            .conversation
            .active_session
            .as_ref()
            .map(|s| s.tab_id);
        let available_names: Vec<&str> = self
            .state
            .conversation
            .connected_sessions
            .iter()
            .filter(|s| Some(s.tab_id) != primary_tab)
            .map(|s| s.host_name.as_str())
            .collect();
        for mentioned in extract_host_mentions(&prompt, &available_names) {
            if let Some(found) = self
                .state
                .conversation
                .connected_sessions
                .iter()
                .find(|s| s.host_name == mentioned && Some(s.tab_id) != primary_tab)
            {
                self.state
                    .conversation
                    .extra_scope_tabs
                    .insert(found.tab_id);
            }
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        let local_item_id = format!("local-u-{run_id}");
        self.state.conversation.items.push(UiChatItem::UserText {
            id: local_item_id.clone(),
            text: prompt.clone(),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        });
        self.active_nav_dot = None;
        self.state.view.status_error = None;
        self.state.run.active_focus = Some(rust_i18n::t!("ai.working_default").to_string());

        let effective_model_id = if let Some(acp_id) = &self.state.config.active_acp_agent_id {
            self.state
                .config
                .settings
                .acp_agents
                .iter()
                .find(|a| &a.id == acp_id)
                .map(|a| format!("acp:{}", a.default_model))
                .unwrap_or_else(|| self.state.config.selected_model_id.clone())
        } else {
            self.state.config.selected_model_id.clone()
        };

        let conversation_id = self.state.conversation.active_conversation_id.clone();
        let req = AiPromptRequest {
            run_id: run_id.clone(),
            conversation_id,
            tab_id: self
                .state
                .conversation
                .active_session
                .as_ref()
                .map(|s| s.tab_id),
            session_id: self
                .state
                .conversation
                .active_session
                .as_ref()
                .map(|s| s.session_id),
            host_name: self
                .state
                .conversation
                .active_session
                .as_ref()
                .map(|s| s.host_name.clone()),
            login: self
                .state
                .conversation
                .active_session
                .as_ref()
                .map(|s| s.login.clone()),
            platform: self
                .state
                .conversation
                .active_session
                .as_ref()
                .map(|s| s.platform.clone()),
            protection: self
                .state
                .conversation
                .active_session
                .as_ref()
                .map(|s| s.protection)
                .unwrap_or(HostProtection::Normal),
            model_id: effective_model_id,
            effort: self
                .state
                .config
                .selected_effort
                .map(|e| e.as_str().to_owned()),
            prompt,
            images_base64,
            attachment_text,
            extra_hosts: self
                .state
                .conversation
                .connected_sessions
                .iter()
                .filter(|s| self.state.conversation.extra_scope_tabs.contains(&s.tab_id))
                .map(|s| AiHostTargetDto {
                    tab_id: s.tab_id,
                    session_id: s.session_id,
                    host_name: s.host_name.clone(),
                    login: s.login.clone(),
                    platform: s.platform.clone(),
                    protection: s.protection,
                })
                .collect(),
            acp_agent_id: self.state.config.active_acp_agent_id.clone(),
            plan_mode: false,
        };
        if let Err(err) = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::SendPrompt(Box::new(
                req,
            )))))
        {
            self.state.conversation.items.retain(
                |item| !matches!(item, UiChatItem::UserText { id, .. } if id == &local_item_id),
            );
            self.state.run.active_focus = None;
            self.state.view.status_error =
                Some(rust_i18n::t!("ai.send_failed", error = err.to_string()).to_string());
        } else {
            self.state.begin_run(
                run_id,
                self.state.conversation.active_conversation_id.clone(),
            );
        }
        cx.notify();
    }

    pub(super) fn send_from_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer_input.read(cx).value().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() && self.state.composer.pending_attachments.is_empty() {
            return;
        }
        let combined = trimmed.to_owned();
        self.composer_input
            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.last_composer_len = 0;
        self.state.view.active_popup = AiActivePopup::None;
        if combined == "/clear" {
            self.clear_conversation(cx);
            return;
        }
        if let Some(rest) = combined.strip_prefix("/compact") {
            self.compact_conversation(rest.trim().to_owned(), cx);
            return;
        }
        self.send_prompt_text(combined, cx);
    }

    pub(super) fn clear_conversation(&mut self, cx: &mut Context<Self>) {
        self.active_nav_dot = None;
        self.editing_title = false;
        self.state.conversation.active_conversation_id = None;
        self.state.conversation.active_conversation_title = None;
        self.state.conversation.items.clear();
        self.refresh_message_block_cache();
        self.state.conversation.metrics.reset_session();
        self.state.run.active_plan = None;
        self.state.run.active_focus = None;
        self.state.view.status_error = None;
        self.state.view.drawer = AiDrawerTab::Chat;
        self.state.view.active_popup = AiActivePopup::None;
        cx.notify();
    }

    /// 供终端右键菜单调用：直接请求解释最近的终端命令块或屏幕输出。
    pub(crate) fn explain_terminal_output(&mut self, snippet: String, cx: &mut Context<Self>) {
        self.state.view.drawer = AiDrawerTab::Chat;
        let trimmed = snippet.trim();
        let prompt = if trimmed.is_empty() {
            rust_i18n::t!("ai.explain_prompt_empty").to_string()
        } else {
            rust_i18n::t!("ai.explain_prompt_with_output", output = trimmed).to_string()
        };
        self.send_prompt_text(prompt, cx);
    }

    /// 供终端右键菜单调用：将选区或失败命令块作为紧凑上下文芯片挂载到 AI 输入区。
    pub(crate) fn attach_terminal_context(
        &mut self,
        host: String,
        cwd: Option<String>,
        exit_code: Option<i32>,
        command: Option<String>,
        output: String,
        cx: &mut Context<Self>,
    ) {
        self.state.view.drawer = AiDrawerTab::Chat;
        self.state.composer.pending_attachments.push(
            super::attachments::ComposerAttachment::TerminalContext {
                id: uuid::Uuid::new_v4().to_string(),
                host,
                cwd,
                exit_code,
                command,
                output,
            },
        );
        cx.notify();
    }

    pub(super) fn save_snippet_from_card(
        &mut self,
        name: String,
        command: String,
        cx: &mut Context<Self>,
    ) {
        cx.emit(AiPanelEvent::SaveSnippetRequested { name, command });
    }
}
