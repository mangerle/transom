//! AI 面板设置抽屉专属表单状态（供应商、模型草稿表、ACP、语言与运行消息行为）。

use gpui_kit::component::input::InputState;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::*;
use transom_ai_contracts::{AiSettings, ProviderConfig, RunningMessageBehavior};

use super::config_drawer::{ConfigChoice, ConfigSelect, DraftModelRow};
use super::model_table::make_draft_model_row;
use super::view::AiAssistantView;

/// 设置抽屉的独立表单状态。
pub(super) struct ConfigFormState {
    pub(super) provider_name_input: Entity<InputState>,
    pub(super) api_key_input: Entity<InputState>,
    pub(super) base_url_input: Entity<InputState>,
    pub(super) draft_models: Vec<DraftModelRow>,
    pub(super) acp_cmd_input: Entity<InputState>,
    pub(super) selected_acp_idx: usize,
    pub(super) lang_select: Entity<ConfigSelect>,
    pub(super) running_behavior_select: Entity<ConfigSelect>,
    /// 正在编辑的完整供应商快照，用于保存时回写未在表单中展示的能力字段。
    pub(super) editing_provider: Option<ProviderConfig>,
}

impl ConfigFormState {
    /// 创建空表单；配置快照到达后再调用 [`Self::apply_settings`] 回填。
    ///
    /// 该分离避免 UI 构造阶段同步读取配置或钥匙串，也确保表单只显示控制平面确认过的快照。
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<AiAssistantView>,
    ) -> (Self, Vec<Subscription>) {
        let provider_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.cfg_name_placeholder").to_string())
        });
        let api_key_input = cx.new(|cx| InputState::new(window, cx).placeholder("sk-..."));
        let base_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        let acp_cmd_input = cx.new(|cx| InputState::new(window, cx).placeholder("claude-code-acp"));

        let lang_select = cx.new(|cx| {
            let items = vec![
                ConfigChoice::new("zh", rust_i18n::t!("ai.cfg_lang_zh").to_string()),
                ConfigChoice::new("en", rust_i18n::t!("ai.cfg_lang_en").to_string()),
                ConfigChoice::new("auto", rust_i18n::t!("ai.cfg_lang_auto").to_string()),
            ];
            SelectState::new(SearchableVec::new(items), None, window, cx).searchable(false)
        });
        let running_behavior_select = cx.new(|cx| {
            let items = vec![
                ConfigChoice::new("queue", rust_i18n::t!("ai.running_msg_queue").to_string()),
                ConfigChoice::new("steer", rust_i18n::t!("ai.running_msg_steer").to_string()),
                ConfigChoice::new(
                    "interrupt",
                    rust_i18n::t!("ai.running_msg_interrupt").to_string(),
                ),
            ];
            SelectState::new(SearchableVec::new(items), None, window, cx).searchable(false)
        });

        let lang_sub = cx.subscribe_in(
            &lang_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ConfigChoice>>, _, cx| {
                if let SelectEvent::Confirm(Some(val)) = event {
                    this.state.config.settings.response_language = val.clone();
                    this.persist_settings_reported(cx);
                }
            },
        );
        let beh_sub = cx.subscribe_in(
            &running_behavior_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ConfigChoice>>, _, cx| {
                if let SelectEvent::Confirm(Some(val)) = event {
                    let behavior = match val.as_str() {
                        "steer" => RunningMessageBehavior::Steer,
                        "interrupt" => RunningMessageBehavior::Interrupt,
                        _ => RunningMessageBehavior::Queue,
                    };
                    this.set_running_message_behavior(behavior, cx);
                }
            },
        );

        let form = Self {
            provider_name_input,
            api_key_input,
            base_url_input,
            draft_models: Vec::new(),
            acp_cmd_input,
            selected_acp_idx: 0,
            lang_select,
            running_behavior_select,
            editing_provider: None,
        };
        (form, vec![lang_sub, beh_sub])
    }

    /// 使用控制平面快照重建表单中的输入控件与供应商草稿。
    ///
    /// API Key 输入框始终以空值开始，已保存密钥只由控制平面在需要时读取：
    /// 这样 GPUI 主线程不访问 Keyring，也不会把明文凭据重新回填到界面。
    pub(super) fn apply_settings(
        &mut self,
        settings: &AiSettings,
        window: &mut Window,
        cx: &mut Context<AiAssistantView>,
    ) -> Result<(), String> {
        self.draft_models.clear();
        let Some(provider) = settings.providers.first() else {
            self.provider_name_input
                .update(cx, |input, cx| input.set_value(String::new(), window, cx));
            self.base_url_input
                .update(cx, |input, cx| input.set_value(String::new(), window, cx));
            self.api_key_input
                .update(cx, |input, cx| input.set_value(String::new(), window, cx));
            self.draft_models.push(make_draft_model_row(
                "",
                "",
                Some(128_000),
                None,
                window,
                cx,
            ));
            self.editing_provider = None;
            self.apply_selects(settings, window, cx);
            return Ok(());
        };

        self.editing_provider = Some(provider.clone());
        let provider_name = provider.name.clone();
        let base_url = provider.base_url.clone();
        self.provider_name_input
            .update(cx, |input, cx| input.set_value(provider_name, window, cx));
        self.base_url_input
            .update(cx, |input, cx| input.set_value(base_url, window, cx));
        self.api_key_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.draft_models
            .extend(provider.models.iter().map(|model| {
                make_draft_model_row(
                    &model.display_name,
                    &model.name,
                    model.context_window,
                    model.efforts.first().copied(),
                    window,
                    cx,
                )
            }));
        if self.draft_models.is_empty() {
            self.draft_models.push(make_draft_model_row(
                "",
                "",
                Some(128_000),
                None,
                window,
                cx,
            ));
        }

        self.selected_acp_idx = settings
            .active_acp_agent_id
            .as_deref()
            .and_then(|active| {
                transom_ai_contracts::default_acp_presets()
                    .iter()
                    .position(|preset| preset.id == active)
            })
            .unwrap_or(0);
        let acp_command = settings
            .active_acp_agent_id
            .as_deref()
            .and_then(|active| settings.acp_agents.iter().find(|agent| agent.id == active))
            .map(format_acp_command)
            .unwrap_or_default();
        self.acp_cmd_input
            .update(cx, |input, cx| input.set_value(acp_command, window, cx));
        self.apply_selects(settings, window, cx);
        Ok(())
    }

    fn apply_selects(
        &self,
        settings: &AiSettings,
        window: &mut Window,
        cx: &mut Context<AiAssistantView>,
    ) {
        let language = settings.response_language.clone();
        self.lang_select.update(cx, |select, cx| {
            select.set_selected_value(&language, window, cx)
        });
        let behavior = match settings.running_message_behavior {
            RunningMessageBehavior::Queue => "queue",
            RunningMessageBehavior::Steer => "steer",
            RunningMessageBehavior::Interrupt => "interrupt",
        };
        self.running_behavior_select.update(cx, |select, cx| {
            select.set_selected_value(&behavior.to_owned(), window, cx)
        });
    }
}

fn format_acp_command(agent: &transom_ai_contracts::AcpAgentConfig) -> String {
    if agent.args.is_empty() {
        agent.command.clone()
    } else {
        format!("{} {}", agent.command, agent.args.join(" "))
    }
}
