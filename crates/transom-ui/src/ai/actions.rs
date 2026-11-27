//! AI 助手面板交互动作与配置/记忆持久化方法。

use gpui_kit::*;
use transom_ai::{
    ExecutionMode, ModelConfig, ProviderConfig, ThinkingEffort, default_acp_presets,
    extract_host_mentions,
};
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
        if let Err(err) = self.ai_store.set_conversation_archived(conv_id, archived) {
            self.state.status_error = Some(err.to_string());
            cx.notify();
            return;
        }
        if let Some(conv) = self.state.conversations.iter_mut().find(|c| c.id == conv_id) {
            conv.archived = archived;
        }
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
            if let Err(err) = self.ai_store.rename_conversation(&cid, &next) {
                self.state.status_error = Some(err.to_string());
            } else {
                self.state.active_conversation_title = Some(next.clone());
                if let Some(conv) = self.state.conversations.iter_mut().find(|c| c.id == cid) {
                    conv.title = next;
                }
            }
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
        if trimmed.is_empty()
            && self.state.composer_chips.is_empty()
            && self.state.pending_attachments.is_empty()
        {
            return;
        }
        let chips = std::mem::take(&mut self.state.composer_chips);
        let mut parts: Vec<String> = chips.into_iter().map(|c| c.label().to_owned()).collect();
        if !trimmed.is_empty() {
            parts.push(text);
        }
        let combined = parts.join(" ");
        let combined_trimmed = combined.trim();
        self.composer_input
            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.last_composer_len = 0;
        self.state.active_popup = AiActivePopup::None;
        if combined_trimmed == "/clear" {
            self.clear_conversation(cx);
            return;
        }
        if let Some(rest) = combined_trimmed.strip_prefix("/compact") {
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
        self.state.composer_chips.clear();
        self.state.metrics.reset_session();
        self.state.active_plan = None;
        self.state.active_focus = None;
        self.state.status_error = None;
        self.state.drawer = AiDrawerTab::Chat;
        self.state.active_popup = AiActivePopup::None;
        cx.notify();
    }

    pub(super) fn save_provider_from_drawer(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name_val = self.provider_name_input.read(cx).value().to_string();
        let base_url = self.base_url_input.read(cx).value().to_string();
        let api_key = self.api_key_input.read(cx).value().to_string();

        let protocol = self.state.selected_protocol;
        let display_name = if name_val.trim().is_empty() {
            protocol.label().to_owned()
        } else {
            name_val.trim().to_owned()
        };
        let slug: String = display_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let prov_id = if slug.is_empty() {
            protocol.as_str().to_owned()
        } else {
            slug.to_lowercase()
        };

        let mut models = Vec::new();
        for row in &self.draft_models {
            let req = row.request_model.read(cx).value().trim().to_owned();
            if req.is_empty() {
                continue;
            }
            let disp = row.display_name.read(cx).value().trim().to_owned();
            let ctx_win = row
                .context_window
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .ok()
                .or(Some(128_000));
            models.push(ModelConfig {
                id: format!("{prov_id}:{req}"),
                display_name: disp,
                name: req,
                vision: true,
                efforts: row.effort.into_iter().collect(),
                max_tokens: None,
                context_window: ctx_win,
            });
        }

        if let Some(first) = models.first()
            && !models.iter().any(|m| m.id == self.state.selected_model_id)
        {
            self.state.selected_model_id = first.id.clone();
            self.state.settings.default_model = first.id.clone();
        }

        let provider = ProviderConfig {
            id: prov_id.clone(),
            name: display_name,
            protocol,
            base_url: base_url.trim().to_owned(),
            thinking_switch: Default::default(),
            secret_key_id: format!("ai-provider:{prov_id}"),
            models,
        };

        if !api_key.trim().is_empty()
            && let Err(err) = self.ai_store.save_provider_api_key(
                self.secret_store.as_ref(),
                &provider.id,
                &api_key,
            )
        {
            self.state.status_error =
                Some(rust_i18n::t!("ai.save_key_failed", error = err.to_string()).to_string());
            cx.notify();
            return;
        }

        self.state
            .settings
            .providers
            .retain(|p| p.id != provider.id);
        self.state.settings.providers.push(provider);
        if let Err(err) = self.ai_store.save_settings(&self.state.settings) {
            self.state.status_error =
                Some(rust_i18n::t!("ai.save_config_failed", error = err.to_string()).to_string());
        } else {
            self.state.status_error = None;
            self.state.drawer = AiDrawerTab::Chat;
        }
        cx.notify();
    }

    pub(super) fn delete_provider(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        self.state
            .settings
            .providers
            .retain(|p| p.id != provider_id);
        if self
            .state
            .selected_model_id
            .starts_with(&format!("{provider_id}:"))
        {
            self.state.selected_model_id = self
                .state
                .settings
                .models()
                .next()
                .map(|(_, m)| m.id.clone())
                .unwrap_or_default();
            self.state.settings.default_model = self.state.selected_model_id.clone();
        }
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn save_memory_from_drawer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let global_md = self.global_mem_input.read(cx).value().to_string();
        if let Err(err) = self.ai_store.save_global_memory(&global_md) {
            self.state.status_error = Some(err.to_string());
        } else {
            self.state.memory.global_markdown = global_md;
        }
        if let Some(session) = &self.state.active_session {
            let host_md = self.host_mem_input.read(cx).value().to_string();
            let sid = session.session_id.to_string();
            if let Err(err) = self.ai_store.save_host_memory(&sid, &host_md) {
                self.state.status_error = Some(err.to_string());
            } else {
                self.state.memory.host_markdown = host_md;
            }
        }
        self.state.drawer = AiDrawerTab::Chat;
        cx.notify();
    }

    pub(super) fn toggle_popup(&mut self, popup: AiActivePopup, cx: &mut Context<Self>) {
        self.state.active_popup =
            if self.state.active_popup == popup {
                AiActivePopup::None
            } else {
                if popup == AiActivePopup::Model
                    && let Some(agent_id) = self.state.active_acp_agent_id.clone()
                {
                    let _ = self.command_tx.try_send(AppCommand::Ai(Box::new(
                        AiCommand::ProbeAcpModels { agent_id },
                    )));
                }
                popup
            };
        cx.notify();
    }

    pub(super) fn select_execution_mode(&mut self, mode: ExecutionMode, cx: &mut Context<Self>) {
        self.state.execution_mode = mode;
        self.state.active_popup = AiActivePopup::None;
        let mode_str = match mode {
            ExecutionMode::Manual => "manual",
            ExecutionMode::SafeReadAuto => "safe_read_auto",
            ExecutionMode::Auto => "auto",
            ExecutionMode::AllowAll => "allow_all",
        };
        self.state.settings.execution_mode = mode;
        let _ = self.ai_store.save_settings(&self.state.settings);
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::SetExecutionMode {
                mode: mode_str.to_owned(),
            })));
        cx.notify();
    }

    pub(super) fn select_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        if let Some(acp_id) = self.state.active_acp_agent_id.clone() {
            if !self
                .state
                .settings
                .acp_agents
                .iter()
                .any(|a| a.id == acp_id)
                && let Some(preset) = default_acp_presets().into_iter().find(|p| p.id == acp_id)
            {
                self.state.settings.acp_agents.push(preset);
            }
            if let Some(agent) = self
                .state
                .settings
                .acp_agents
                .iter_mut()
                .find(|a| a.id == acp_id)
            {
                agent.default_model = model_id;
            }
        } else {
            self.state.selected_model_id = model_id.clone();
            self.state.settings.default_model = model_id;
        }
        self.state.active_popup = AiActivePopup::None;
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn select_effort(&mut self, effort: Option<ThinkingEffort>, cx: &mut Context<Self>) {
        self.state.selected_effort = effort;
        self.state.settings.default_effort = effort;
        let _ = self.ai_store.save_settings(&self.state.settings);
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

    /// 优先使用输入框当前填写的 API Key；为空时从系统密钥库回读匹配供应商的已保存密钥。
    fn resolve_drawer_api_key(&self, cx: &App) -> String {
        let typed = self.api_key_input.read(cx).value().trim().to_owned();
        if !typed.is_empty() {
            return typed;
        }
        let name = self.provider_name_input.read(cx).value().trim().to_owned();
        let url = self.base_url_input.read(cx).value().trim().to_owned();
        let matched = self
            .state
            .settings
            .providers
            .iter()
            .find(|p| (!name.is_empty() && p.name == name) || (!url.is_empty() && p.base_url == url))
            .or_else(|| self.state.settings.providers.first());
        if let Some(prov) = matched
            && let Ok(Some(saved)) = self
                .ai_store
                .load_provider_api_key(self.secret_store.as_ref(), &prov.id)
        {
            return saved.to_string();
        }
        String::new()
    }

    pub(super) fn test_provider_connection(&mut self, cx: &mut Context<Self>) {
        let base_url = self.base_url_input.read(cx).value().to_string();
        let api_key = self.resolve_drawer_api_key(cx);
        self.state.discovery_status = Some(rust_i18n::t!("ai.testing_connection").to_string());
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::TestConnection {
                request_id: uuid::Uuid::new_v4().to_string(),
                protocol: self.state.selected_protocol.as_str().to_owned(),
                base_url,
                api_key,
            })));
        cx.notify();
    }

    pub(super) fn discover_provider_models(&mut self, cx: &mut Context<Self>) {
        let base_url = self.base_url_input.read(cx).value().to_string();
        let api_key = self.resolve_drawer_api_key(cx);
        self.state.discovery_status = Some(rust_i18n::t!("ai.fetching_models").to_string());
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::DiscoverModels {
                request_id: uuid::Uuid::new_v4().to_string(),
                protocol: self.state.selected_protocol.as_str().to_owned(),
                base_url,
                api_key,
            })));
        cx.notify();
    }
}
