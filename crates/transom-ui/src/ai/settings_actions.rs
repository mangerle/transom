//! AI 助手面板配置、供应商、ACP、记忆与模型发现动作。

use gpui_kit::*;
use transom_ai::{
    ExecutionMode, ModelConfig, ProviderConfig, ThinkingEffort, default_acp_presets,
    new_provider_id, provider_model_id,
};
use transom_core::{AiCommand, AppCommand};

use super::state::{AiActivePopup, AiDrawerTab};
use super::view::AiAssistantView;

impl AiAssistantView {
    /// 通过控制平面异步持久化当前 AI 设置，避免阻塞 GPUI 主线程。
    pub(super) fn persist_settings(&mut self) -> Result<(), String> {
        let settings_json =
            serde_json::to_string(&self.state.config.settings).map_err(|err| err.to_string())?;
        let request_id = uuid::Uuid::new_v4().to_string();
        self.command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::SaveSettingsJson {
                request_id: request_id.clone(),
                settings_json,
            })))
            .map_err(|err| err.to_string())?;
        self.state.config.pending_settings_save = Some(request_id);
        Ok(())
    }

    /// 发送配置保存命令；发送失败或序列化失败时立即在当前面板展示中文错误。
    pub(super) fn persist_settings_reported(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = self.persist_settings() {
            self.state.view.status_error =
                Some(rust_i18n::t!("ai.save_config_failed", error = err).to_string());
        }
        cx.notify();
    }

    /// 请求控制平面探测当前 ACP Agent 的可用模型。
    pub(super) fn probe_active_acp_agent(&mut self, cx: &mut Context<Self>) {
        let Some(agent_id) = self.state.config.active_acp_agent_id.clone() else {
            return;
        };
        if let Err(err) =
            self.command_tx
                .try_send(AppCommand::Ai(Box::new(AiCommand::ProbeAcpModels {
                    agent_id,
                })))
        {
            self.state.view.status_error =
                Some(rust_i18n::t!("ai.probe_acp_failed", error = err.to_string()).to_string());
        }
        cx.notify();
    }

    pub(super) fn save_provider_from_drawer(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name_val = self
            .config_form
            .provider_name_input
            .read(cx)
            .value()
            .to_string();
        let base_url = self.config_form.base_url_input.read(cx).value().to_string();
        let api_key = self.config_form.api_key_input.read(cx).value().to_string();

        let protocol = self.state.config.selected_protocol;
        let display_name = if name_val.trim().is_empty() {
            protocol.label().to_owned()
        } else {
            name_val.trim().to_owned()
        };
        let prov_id = self
            .config_form
            .editing_provider
            .as_ref()
            .map(|provider| provider.id.clone())
            .unwrap_or_else(new_provider_id);
        let existing_provider = self.config_form.editing_provider.as_ref();

        let mut models = Vec::new();
        for row in &self.config_form.draft_models {
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
            let existing_model = existing_provider.and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.name == req)
                    .cloned()
            });
            let efforts = match &existing_model {
                Some(model)
                    if model.efforts.first().copied() == row.effort && model.efforts.len() > 1 =>
                {
                    model.efforts.clone()
                }
                _ => row.effort.into_iter().collect(),
            };
            models.push(ModelConfig {
                id: existing_model
                    .as_ref()
                    .map(|model| model.id.clone())
                    .unwrap_or_else(|| provider_model_id(&prov_id, &req)),
                display_name: disp,
                name: req,
                vision: existing_model.as_ref().is_none_or(|model| model.vision),
                efforts,
                max_tokens: existing_model.and_then(|model| model.max_tokens),
                context_window: ctx_win,
            });
        }

        if let Some(first) = models.first()
            && !models
                .iter()
                .any(|m| m.id == self.state.config.selected_model_id)
        {
            self.state.config.selected_model_id = first.id.clone();
            self.state.config.settings.default_model = first.id.clone();
        }

        let provider = ProviderConfig {
            id: prov_id.clone(),
            name: display_name,
            protocol,
            base_url: base_url.trim().to_owned(),
            thinking_switch: existing_provider
                .map(|provider| provider.thinking_switch)
                .unwrap_or_default(),
            models,
        };

        if !api_key.trim().is_empty()
            && let Err(err) = self.ai_store.save_provider_api_key(
                self.secret_store.as_ref(),
                &provider.id,
                &api_key,
            )
        {
            self.state.view.status_error =
                Some(rust_i18n::t!("ai.save_key_failed", error = err.to_string()).to_string());
            cx.notify();
            return;
        }

        self.state
            .config
            .settings
            .providers
            .retain(|p| p.id != provider.id);
        self.config_form.editing_provider = Some(provider.clone());
        self.state.config.settings.providers.push(provider);
        if let Err(err) = self.persist_settings() {
            self.state.view.status_error =
                Some(rust_i18n::t!("ai.save_config_failed", error = err).to_string());
        } else {
            self.state.view.status_error = None;
            self.state.view.drawer = AiDrawerTab::Chat;
        }
        cx.notify();
    }

    pub(super) fn delete_provider(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        if self
            .config_form
            .editing_provider
            .as_ref()
            .is_some_and(|provider| provider.id == provider_id)
        {
            self.config_form.editing_provider = None;
        }
        self.state
            .config
            .settings
            .providers
            .retain(|p| p.id != provider_id);
        if self
            .state
            .config
            .selected_model_id
            .starts_with(&format!("{provider_id}:"))
        {
            self.state.config.selected_model_id = self
                .state
                .config
                .settings
                .models()
                .next()
                .map(|(_, m)| m.id.clone())
                .unwrap_or_default();
            self.state.config.settings.default_model = self.state.config.selected_model_id.clone();
        }
        self.persist_settings_reported(cx);
    }

    pub(super) fn save_memory_from_drawer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let global_md = self
            .memory_form
            .global_mem_input
            .read(cx)
            .value()
            .to_string();
        self.state.memory.global_markdown = global_md.clone();
        let (host_id, host_markdown) =
            if let Some(session) = &self.state.conversation.active_session {
                let host_md = self.memory_form.host_mem_input.read(cx).value().to_string();
                self.state.memory.host_markdown = host_md.clone();
                (Some(session.session_id.to_string()), Some(host_md))
            } else {
                (None, None)
            };
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::SaveMemory {
                host_id,
                global_markdown: Some(global_md),
                host_markdown,
            })));
        self.state.view.drawer = AiDrawerTab::Chat;
        cx.notify();
    }

    pub(super) fn toggle_popup(&mut self, popup: AiActivePopup, cx: &mut Context<Self>) {
        self.state.view.active_popup = if self.state.view.active_popup == popup {
            AiActivePopup::None
        } else {
            if popup == AiActivePopup::Model && self.state.config.active_acp_agent_id.is_some() {
                self.probe_active_acp_agent(cx);
            }
            popup
        };
        cx.notify();
    }

    pub(super) fn select_execution_mode(&mut self, mode: ExecutionMode, cx: &mut Context<Self>) {
        self.state.config.execution_mode = mode;
        self.state.view.active_popup = AiActivePopup::None;
        self.state.config.settings.execution_mode = mode;
        self.persist_settings_reported(cx);
    }

    pub(super) fn select_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        if let Some(acp_id) = self.state.config.active_acp_agent_id.clone() {
            if !self
                .state
                .config
                .settings
                .acp_agents
                .iter()
                .any(|a| a.id == acp_id)
                && let Some(preset) = default_acp_presets().into_iter().find(|p| p.id == acp_id)
            {
                self.state.config.settings.acp_agents.push(preset);
            }
            if let Some(agent) = self
                .state
                .config
                .settings
                .acp_agents
                .iter_mut()
                .find(|a| a.id == acp_id)
            {
                agent.default_model = model_id;
            }
        } else {
            self.state.config.selected_model_id = model_id.clone();
            self.state.config.settings.default_model = model_id;
        }
        self.state.view.active_popup = AiActivePopup::None;
        self.persist_settings_reported(cx);
    }

    pub(super) fn select_effort(&mut self, effort: Option<ThinkingEffort>, cx: &mut Context<Self>) {
        self.state.config.selected_effort = effort;
        self.state.config.settings.default_effort = effort;
        self.persist_settings_reported(cx);
    }

    /// 返回输入框中的临时密钥和可用于控制平面回读 Keyring 的供应商 ID。
    ///
    /// UI 主线程不直接访问 Keyring；输入框为空时只传递供应商 ID，由控制平面
    /// 在阻塞线程池读取已保存密钥。
    fn resolve_drawer_credentials(&self, cx: &App) -> (String, Option<String>) {
        let typed = self
            .config_form
            .api_key_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let name = self
            .config_form
            .provider_name_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let url = self
            .config_form
            .base_url_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let provider_id = self
            .state
            .config
            .settings
            .providers
            .iter()
            .find(|provider| {
                (!name.is_empty() && provider.name == name)
                    || (!url.is_empty() && provider.base_url == url)
            })
            .or_else(|| self.state.config.settings.providers.first())
            .map(|provider| provider.id.clone());
        (typed, provider_id)
    }

    pub(super) fn test_provider_connection(&mut self, cx: &mut Context<Self>) {
        let base_url = self.config_form.base_url_input.read(cx).value().to_string();
        let (api_key, provider_id) = self.resolve_drawer_credentials(cx);
        self.state.config.discovery_status =
            Some(rust_i18n::t!("ai.testing_connection").to_string());
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::TestConnection {
                request_id: uuid::Uuid::new_v4().to_string(),
                provider_id,
                protocol: self.state.config.selected_protocol.as_str().to_owned(),
                base_url,
                api_key,
            })));
        cx.notify();
    }

    pub(super) fn discover_provider_models(&mut self, cx: &mut Context<Self>) {
        let base_url = self.config_form.base_url_input.read(cx).value().to_string();
        let (api_key, provider_id) = self.resolve_drawer_credentials(cx);
        self.state.config.discovery_status = Some(rust_i18n::t!("ai.fetching_models").to_string());
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::DiscoverModels {
                request_id: uuid::Uuid::new_v4().to_string(),
                provider_id,
                protocol: self.state.config.selected_protocol.as_str().to_owned(),
                base_url,
                api_key,
            })));
        cx.notify();
    }

    pub(super) fn apply_acp_preset(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let presets = default_acp_presets();
        let Some(agent) = presets.get(idx) else {
            return;
        };
        self.config_form.selected_acp_idx = idx;
        let line = if agent.args.is_empty() {
            agent.command.clone()
        } else {
            format!("{} {}", agent.command, agent.args.join(" "))
        };
        self.config_form
            .acp_cmd_input
            .update(cx, |i, cx| i.set_value(line, window, cx));
        cx.notify();
    }

    pub(super) fn save_acp_from_drawer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let presets = default_acp_presets();
        let base = presets
            .get(self.config_form.selected_acp_idx)
            .cloned()
            .unwrap_or_default();
        let raw_cmd = self.config_form.acp_cmd_input.read(cx).value().to_string();
        let parts: Vec<String> = raw_cmd.split_whitespace().map(str::to_owned).collect();
        let Some(cmd) = parts.first().cloned() else {
            return;
        };
        let mut agent = base;
        agent.command = cmd;
        agent.args = parts.into_iter().skip(1).collect();
        agent.enabled = true;
        let agent_id = agent.id.clone();
        self.state
            .config
            .settings
            .acp_agents
            .retain(|a| a.id != agent_id);
        self.state.config.settings.acp_agents.push(agent);
        self.state.config.settings.active_acp_agent_id = Some(agent_id.clone());
        self.state.config.active_acp_agent_id = Some(agent_id);
        self.persist_settings_reported(cx);
        self.state.view.drawer = AiDrawerTab::Chat;
        cx.notify();
    }

    pub(super) fn select_acp_agent(&mut self, agent_id: Option<String>, cx: &mut Context<Self>) {
        if let Some(id) = &agent_id
            && !self
                .state
                .config
                .settings
                .acp_agents
                .iter()
                .any(|a| &a.id == id)
            && let Some(mut preset) = default_acp_presets().into_iter().find(|p| &p.id == id)
        {
            preset.enabled = true;
            self.state.config.settings.acp_agents.push(preset);
        }
        self.state.config.active_acp_agent_id = agent_id.clone();
        self.state.config.settings.active_acp_agent_id = agent_id.clone();
        self.state.view.active_popup = AiActivePopup::None;
        self.persist_settings_reported(cx);
        if agent_id.is_some() {
            self.probe_active_acp_agent(cx);
        }
    }
}
