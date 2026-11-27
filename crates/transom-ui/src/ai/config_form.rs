//! AI 面板设置抽屉专属表单状态（供应商、模型草稿表、ACP、语言与运行消息行为）。

use gpui_kit::component::input::InputState;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::*;
use transom_ai::{AiSettings, RunningMessageBehavior};
use transom_core::SecretStore;
use transom_store::AiStore;

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
}

impl ConfigFormState {
    pub(super) fn new(
        settings: &AiSettings,
        ai_store: &AiStore,
        secret_store: &dyn SecretStore,
        window: &mut Window,
        cx: &mut Context<AiAssistantView>,
    ) -> (Self, Vec<Subscription>) {
        let provider_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.cfg_name_placeholder").to_string())
        });
        let api_key_input = cx.new(|cx| InputState::new(window, cx).placeholder("sk-..."));
        let base_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        let mut draft_models = Vec::new();
        if let Some(first_prov) = settings.providers.first() {
            let pname = first_prov.name.clone();
            let purl = first_prov.base_url.clone();
            provider_name_input.update(cx, |i, cx| i.set_value(pname, window, cx));
            base_url_input.update(cx, |i, cx| i.set_value(purl, window, cx));
            if let Ok(Some(saved_key)) =
                ai_store.load_provider_api_key(secret_store, &first_prov.id)
                && !saved_key.trim().is_empty()
            {
                api_key_input.update(cx, |i, cx| i.set_value(saved_key.as_str(), window, cx));
            }
            for m in &first_prov.models {
                draft_models.push(make_draft_model_row(
                    &m.display_name,
                    &m.name,
                    m.context_window,
                    m.efforts.first().copied(),
                    window,
                    cx,
                ));
            }
        }
        if draft_models.is_empty() {
            draft_models.push(make_draft_model_row(
                "",
                "",
                Some(128_000),
                None,
                window,
                cx,
            ));
        }
        let acp_cmd_input = cx.new(|cx| InputState::new(window, cx).placeholder("claude-code-acp"));

        let cur_lang = settings.response_language.clone();
        let lang_select = cx.new(|cx| {
            let items = vec![
                ConfigChoice::new("zh", rust_i18n::t!("ai.cfg_lang_zh").to_string()),
                ConfigChoice::new("en", rust_i18n::t!("ai.cfg_lang_en").to_string()),
                ConfigChoice::new("auto", rust_i18n::t!("ai.cfg_lang_auto").to_string()),
            ];
            let mut s =
                SelectState::new(SearchableVec::new(items), None, window, cx).searchable(false);
            s.set_selected_value(&cur_lang, window, cx);
            s
        });
        let cur_beh_val = match settings.running_message_behavior {
            RunningMessageBehavior::Queue => "queue",
            RunningMessageBehavior::Steer => "steer",
            RunningMessageBehavior::Interrupt => "interrupt",
        };
        let running_behavior_select = cx.new(|cx| {
            let items = vec![
                ConfigChoice::new("queue", rust_i18n::t!("ai.running_msg_queue").to_string()),
                ConfigChoice::new("steer", rust_i18n::t!("ai.running_msg_steer").to_string()),
                ConfigChoice::new(
                    "interrupt",
                    rust_i18n::t!("ai.running_msg_interrupt").to_string(),
                ),
            ];
            let mut s =
                SelectState::new(SearchableVec::new(items), None, window, cx).searchable(false);
            s.set_selected_value(&cur_beh_val.to_owned(), window, cx);
            s
        });

        let lang_sub = cx.subscribe_in(
            &lang_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ConfigChoice>>, _, cx| {
                if let SelectEvent::Confirm(Some(val)) = event {
                    this.state.settings.response_language = val.clone();
                    let _ = this.persist_settings();
                    cx.notify();
                }
            },
        );
        let beh_sub = cx.subscribe_in(
            &running_behavior_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ConfigChoice>>, _, cx| {
                if let SelectEvent::Confirm(Some(val)) = event {
                    let beh = match val.as_str() {
                        "steer" => RunningMessageBehavior::Steer,
                        "interrupt" => RunningMessageBehavior::Interrupt,
                        _ => RunningMessageBehavior::Queue,
                    };
                    this.set_running_message_behavior(beh, cx);
                }
            },
        );

        let form = Self {
            provider_name_input,
            api_key_input,
            base_url_input,
            draft_models,
            acp_cmd_input,
            selected_acp_idx: 0,
            lang_select,
            running_behavior_select,
        };
        (form, vec![lang_sub, beh_sub])
    }
}
