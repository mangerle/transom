//! AI 配置快照与保存确认的状态迁移。

use transom_ai::AiSettings;

use super::AiPanelState;

impl AiPanelState {
    pub(super) fn handle_settings_saved(&mut self, request_id: String, error: Option<String>) {
        if self.config.pending_settings_save.as_deref() != Some(request_id.as_str()) {
            return;
        }
        self.config.pending_settings_save = None;
        if let Some(err) = error {
            self.view.status_error =
                Some(rust_i18n::t!("ai.save_config_failed", error = err).to_string());
        } else {
            self.view.status_error = None;
        }
    }

    pub(super) fn handle_settings_snapshot(
        &mut self,
        request_id: String,
        settings_json: Option<String>,
        error: Option<String>,
    ) -> Option<AiSettings> {
        if self.config.pending_settings_load.as_deref() != Some(request_id.as_str()) {
            return None;
        }
        self.config.pending_settings_load = None;
        let parsed = settings_json
            .as_deref()
            .map(serde_json::from_str::<AiSettings>)
            .transpose();
        let loaded = match parsed {
            Ok(settings) => settings.inspect(|settings| {
                self.apply_loaded_settings(settings.clone());
            }),
            Err(err) => {
                self.view.status_error = Some(
                    rust_i18n::t!("ai.invalid_config_snapshot", error = err.to_string())
                        .to_string(),
                );
                None
            }
        };
        if let Some(err) = error {
            self.view.status_error =
                Some(rust_i18n::t!("ai.load_config_failed", error = err).to_string());
        }
        loaded
    }

    fn apply_loaded_settings(&mut self, settings: AiSettings) {
        self.config.selected_model_id = settings.default_model.clone();
        self.config.selected_effort = settings.default_effort;
        self.config.execution_mode = settings.execution_mode;
        self.config.active_acp_agent_id = settings.active_acp_agent_id.clone();
        self.config.selected_protocol = settings
            .resolve_model(&self.config.selected_model_id)
            .map(|(provider, _)| provider.protocol)
            .unwrap_or(transom_ai::ProviderProtocol::OpenAi);
        if self.config.selected_model_id.is_empty()
            && let Some((_, model)) = settings.models().next()
        {
            self.config.selected_model_id = model.id.clone();
        }
        self.config.settings = settings;
    }
}
