//! AI 助手面板会话与消息发送交互动作。

use gpui_kit::*;
use transom_ai::extract_host_mentions;
use transom_core::{
    AiCommand, AiHostTargetDto, AiPromptRequest, AppCommand, HostProtection, TabId,
};

use super::AiPanelEvent;
use super::attachments::pack_attachments_for_request;
use super::state::{AiActivePopup, AiDrawerTab, UiChatItem};
use super::view::AiAssistantView;

impl AiAssistantView {

    pub(super) fn select_conversation(&mut self, conv_id: &str) {
        self.active_nav_dot = None;
        self.editing_title = false;
        self.state.active_conversation_id = Some(conv_id.to_owned());
        self.state.active_conversation_title = self
            .state
            .conversations
            .iter()
            .find(|c| c.id == conv_id)
            .map(|c| c.title.clone());
        self.state.metrics.reset_session();
        if let Ok(tree) = self.ai_store.load_session_tree(conv_id) {
            let branch: Vec<_> = tree.active_branch().into_iter().cloned().collect();
            self.state.load_entries(branch);
        }
    }

    pub(super) fn delete_conversation(&mut self, conv_id: &str, cx: &mut Context<Self>) {
        self.state.conversations.retain(|c| c.id != conv_id);
        if self.state.active_conversation_id.as_deref() == Some(conv_id) {
            self.clear_conversation(cx);
        }
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::DeleteConversation {
                conversation_id: conv_id.to_owned(),
            })));
        cx.notify();
    }

    pub(super) fn set_conversation_archived(
        &mut self,
        conv_id: &str,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(conv) = self.state.conversations.iter_mut().find(|c| c.id == conv_id) {
            conv.archived = archived;
        }
        let _ = self.command_tx.try_send(AppCommand::Ai(Box::new(
            AiCommand::SetConversationArchived {
                conversation_id: conv_id.to_owned(),
                archived,
            },
        )));
        cx.notify();
    }

    pub(super) fn start_editing_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.active_conversation_id.is_none() {
            return;
        }
        let cur = self
            .state
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
        if let Some(cid) = self.state.active_conversation_id.clone()
            && !next.is_empty()
        {
            self.state.active_conversation_title = Some(next.clone());
            if let Some(conv) = self.state.conversations.iter_mut().find(|c| c.id == cid) {
                conv.title = next.clone();
            }
            let _ = self.command_tx.try_send(AppCommand::Ai(Box::new(
                AiCommand::RenameConversation {
                    conversation_id: cid,
                    title: next,
                },
            )));
        }
        cx.notify();
    }

    pub(super) fn toggle_extra_scope_tab(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if !self.state.extra_scope_tabs.remove(&tab_id) {
            self.state.extra_scope_tabs.insert(tab_id);
        }
        cx.notify();
    }

    pub(super) fn send_prompt_text(&mut self, raw_prompt: String, cx: &mut Context<Self>) {
        let mut prompt = raw_prompt.trim().to_owned();
        if self.state.running_run_id.is_some() {
            if !prompt.is_empty() || !self.state.pending_attachments.is_empty() {
                self.handle_running_message(prompt, cx);
            }
            return;
        }
        self.state.queue_paused_by_cancel = false;
        self.auto_scroll_locked = false;
        self.active_nav_dot = None;
        self.msg_scroll_handle.scroll_to_bottom();
        let packed = pack_attachments_for_request(&self.state.pending_attachments);
        let images_base64 = packed.images;
        let attachment_text = packed.text;
        if prompt.is_empty() {
            if images_base64.is_empty() && attachment_text.is_none() {
                return;
            }
            prompt = rust_i18n::t!("ai.analyze_attachments_prompt").to_string();
        }
        self.state.pending_attachments.clear();
        let primary_tab = self.state.active_session.as_ref().map(|s| s.tab_id);
        let available_names: Vec<&str> = self
            .state
            .connected_sessions
            .iter()
            .filter(|s| Some(s.tab_id) != primary_tab)
            .map(|s| s.host_name.as_str())
            .collect();
        for mentioned in extract_host_mentions(&prompt, &available_names) {
            if let Some(found) = self
                .state
                .connected_sessions
                .iter()
                .find(|s| s.host_name == mentioned && Some(s.tab_id) != primary_tab)
            {
                self.state.extra_scope_tabs.insert(found.tab_id);
            }
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        self.state.items.push(UiChatItem::UserText {
            id: format!("local-u-{run_id}"),
            text: prompt.clone(),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        });
        self.state.running_run_id = Some(run_id.clone());
        self.active_nav_dot = None;
        self.state.status_error = None;
        self.state.run_started_at = Some(std::time::Instant::now());
        self.state.active_focus = Some(rust_i18n::t!("ai.working_default").to_string());

        let effective_model_id = if let Some(acp_id) = &self.state.active_acp_agent_id {
            self.state
                .settings
                .acp_agents
                .iter()
                .find(|a| &a.id == acp_id)
                .map(|a| format!("acp:{}", a.default_model))
                .unwrap_or_else(|| self.state.selected_model_id.clone())
        } else {
            self.state.selected_model_id.clone()
        };

        let req = AiPromptRequest {
            run_id,
            conversation_id: self.state.active_conversation_id.clone(),
            tab_id: self.state.active_session.as_ref().map(|s| s.tab_id),
            session_id: self.state.active_session.as_ref().map(|s| s.session_id),
            host_name: self
                .state
                .active_session
                .as_ref()
                .map(|s| s.host_name.clone()),
            login: self.state.active_session.as_ref().map(|s| s.login.clone()),
            platform: self
                .state
                .active_session
                .as_ref()
                .map(|s| s.platform.clone()),
            protection: self
                .state
                .active_session
                .as_ref()
                .map(|s| s.protection)
                .unwrap_or(HostProtection::Normal),
            model_id: effective_model_id,
            effort: self.state.selected_effort.map(|e| e.as_str().to_owned()),
            prompt,
            images_base64,
            attachment_text,
            extra_hosts: self
                .state
                .connected_sessions
                .iter()
                .filter(|s| self.state.extra_scope_tabs.contains(&s.tab_id))
                .map(|s| AiHostTargetDto {
                    tab_id: s.tab_id,
                    session_id: s.session_id,
                    host_name: s.host_name.clone(),
                    login: s.login.clone(),
                    platform: s.platform.clone(),
                    protection: s.protection,
                })
                .collect(),
            acp_agent_id: self.state.active_acp_agent_id.clone(),
            plan_mode: false,
        };
        if let Err(err) = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::SendPrompt(Box::new(
                req,
            )))))
        {
            self.state.running_run_id = None;
            self.state.status_error =
                Some(rust_i18n::t!("ai.send_failed", error = err.to_string()).to_string());
        }
        cx.notify();
    }

    pub(super) fn send_from_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer_input.read(cx).value().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() && self.state.pending_attachments.is_empty() {
            return;
        }
        let combined = trimmed.to_owned();
        self.composer_input
            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.last_composer_len = 0;
        self.state.active_popup = AiActivePopup::None;
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
        self.state.active_conversation_id = None;
        self.state.active_conversation_title = None;
        self.state.items.clear();
        self.state.metrics.reset_session();
        self.state.active_plan = None;
        self.state.active_focus = None;
        self.state.status_error = None;
        self.state.drawer = AiDrawerTab::Chat;
        self.state.active_popup = AiActivePopup::None;
        cx.notify();
    }

    /// 供终端右键菜单调用：直接请求解释最近的终端命令块或屏幕输出。
    pub(crate) fn explain_terminal_output(&mut self, snippet: String, cx: &mut Context<Self>) {
        self.state.drawer = AiDrawerTab::Chat;
        let trimmed = snippet.trim();
        let prompt = if trimmed.is_empty() {
            rust_i18n::t!("ai.explain_prompt_empty").to_string()
        } else {
            rust_i18n::t!("ai.explain_prompt_with_output", output = trimmed).to_string()
        };
        self.send_prompt_text(prompt, cx);
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
