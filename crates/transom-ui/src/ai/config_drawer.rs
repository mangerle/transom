//! AI 面板内嵌的模型配置（按协议格式驱动 + 多模型增删表格）与三层记忆编辑抽屉。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::{
    AiSettings, MemorySnapshot, ProviderProtocol, ThinkingEffort, default_acp_presets,
};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 模型配置表中的单行草稿状态。
pub(super) struct DraftModelRow {
    pub(super) display_name: Entity<InputState>,
    pub(super) request_model: Entity<InputState>,
    pub(super) context_window: Entity<InputState>,
    pub(super) effort: Option<ThinkingEffort>,
}

/// 供应商配置抽屉渲染参数。
pub(super) struct ConfigDrawerParams<'a> {
    pub(super) settings: &'a AiSettings,
    pub(super) selected_protocol: ProviderProtocol,
    pub(super) provider_name_input: &'a Entity<InputState>,
    pub(super) api_key_input: &'a Entity<InputState>,
    pub(super) base_url_input: &'a Entity<InputState>,
    pub(super) models_section: AnyElement,
    pub(super) picker_panel: Option<AnyElement>,
    pub(super) has_discovered_models: bool,
    pub(super) acp_cmd_input: &'a Entity<InputState>,
    pub(super) selected_acp_idx: usize,
    pub(super) acp_install_status: Option<&'a (String, bool, String)>,
    pub(super) discovery_status: Option<&'a str>,
    pub(super) p: &'a Palette,
}

pub(super) type ActionCb = Box<dyn Fn(&mut Window, &mut App)>;
pub(super) type IndexCb = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub(super) type ProtocolCb = Rc<dyn Fn(ProviderProtocol, &mut Window, &mut App)>;
pub(super) type ProviderIdCb = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub(super) type LangCb = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// 供应商配置抽屉交互回调。
pub(super) struct ConfigDrawerActions {
    pub(super) on_select_protocol: ProtocolCb,
    pub(super) on_select_language: LangCb,
    pub(super) on_test_connection: ActionCb,
    pub(super) on_fetch_models: ActionCb,
    pub(super) on_open_model_picker: ActionCb,
    pub(super) on_add_model_row: ActionCb,
    pub(super) on_save_provider: ActionCb,
    pub(super) on_edit_provider: ProviderIdCb,
    pub(super) on_delete_provider: ProviderIdCb,
    pub(super) on_apply_acp_preset: IndexCb,
    pub(super) on_save_acp: ActionCb,
    pub(super) on_install_acp: ActionCb,
}

fn field_label(text: String, p: &Palette) -> Div {
    div()
        .text_size(rems(typography::CAPTION))
        .text_color(p.text.muted)
        .child(text)
}

/// 渲染协议驱动的模型配置与 ACP 管理抽屉。
pub(super) fn render_config_drawer(
    params: ConfigDrawerParams<'_>,
    actions: ConfigDrawerActions,
) -> AnyElement {
    let p = params.p;
    let mut proto_row = h_flex().gap(rems(space::XS)).flex_wrap();
    for proto in ProviderProtocol::ALL {
        let active = proto == params.selected_protocol;
        let cb = actions.on_select_protocol.clone();
        let btn = Button::new(SharedString::from(format!("ai-proto-{}", proto.as_str())))
            .label(proto.label());
        let btn = if active { btn.primary() } else { btn.ghost() };
        proto_row = proto_row.child(btn.on_click(move |_, w, cx| cb(proto, w, cx)));
    }

    let mut providers_box = v_flex().gap(rems(space::XS));
    for (idx, prov) in params.settings.providers.iter().enumerate() {
        let models: Vec<&str> = prov.models.iter().map(|m| m.display_label()).collect();
        let edit_cb = actions.on_edit_provider.clone();
        let del_cb = actions.on_delete_provider.clone();
        let edit_id = prov.id.clone();
        let del_id = prov.id.clone();
        let del_tip = rust_i18n::t!("ai.delete_provider").to_string();
        providers_box = providers_box.child(
            h_flex()
                .id(("ai-prov-row", idx))
                .p(rems(space::XS))
                .justify_between()
                .items_center()
                .rounded(p.radius.sm)
                .bg(p.secondary)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, w, cx| edit_cb(edit_id.clone(), w, cx))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.foreground)
                        .child(
                            rust_i18n::t!(
                                "ai.provider_row",
                                name = prov.name.as_str(),
                                protocol = prov.protocol.label(),
                                models = models.join(", ")
                            )
                            .to_string(),
                        ),
                )
                .child(
                    Button::new(("ai-del-prov", idx))
                        .ghost()
                        .accessibility_label(del_tip.clone())
                        .tooltip(del_tip)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                        .on_click(move |_, w, cx| {
                            cx.stop_propagation();
                            del_cb(del_id.clone(), w, cx);
                        }),
                ),
        );
    }

    let mut acp_row = h_flex().gap(rems(space::XS)).flex_wrap();
    let acp_presets = default_acp_presets();
    for (idx, preset) in acp_presets.iter().enumerate() {
        let cb = actions.on_apply_acp_preset.clone();
        let btn = Button::new(SharedString::from(format!("ai-acp-preset-{idx}")))
            .label(preset.name.clone());
        let btn = if idx == params.selected_acp_idx {
            btn.primary()
        } else {
            btn.ghost()
        };
        acp_row = acp_row.child(btn.on_click(move |_, w, cx| cb(idx, w, cx)));
    }

    let mut lang_row = h_flex().gap(rems(space::XXS)).flex_wrap();
    for (idx, (code, label_key)) in [
        ("zh", "ai.cfg_lang_zh"),
        ("en", "ai.cfg_lang_en"),
        ("auto", "ai.cfg_lang_auto"),
    ]
    .into_iter()
    .enumerate()
    {
        let active = params.settings.response_language == code;
        let on_lang = actions.on_select_language.clone();
        let code_owned = code.to_owned();
        let btn = Button::new(("ai-lang", idx)).label(rust_i18n::t!(label_key).to_string());
        let btn = if active { btn.primary() } else { btn.ghost() };
        lang_row = lang_row.child(btn.on_click(move |_, w, cx| on_lang(code_owned.clone(), w, cx)));
    }

    let mut root = v_flex()
        .flex_1()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::SM))
        .overflow_y_scrollbar()
        .child(field_label(
            rust_i18n::t!("ai.cfg_language_label").to_string(),
            p,
        ))
        .child(lang_row)
        .child(field_label(
            rust_i18n::t!("ai.cfg_name_label").to_string(),
            p,
        ))
        .child(Input::new(params.provider_name_input))
        .child(field_label(
            rust_i18n::t!("ai.api_key_label").to_string(),
            p,
        ))
        .child(Input::new(params.api_key_input))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(field_label(
                    rust_i18n::t!("ai.cfg_api_url_label").to_string(),
                    p,
                ))
                .child(
                    Button::new("ai-test-conn")
                        .ghost()
                        .label(rust_i18n::t!("ai.test_connection").to_string())
                        .on_click(move |_, w, cx| (actions.on_test_connection)(w, cx)),
                ),
        )
        .child(Input::new(params.base_url_input))
        .child(field_label(
            rust_i18n::t!("ai.cfg_protocol_label").to_string(),
            p,
        ))
        .child(proto_row)
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(field_label(
                    rust_i18n::t!("ai.cfg_models_title").to_string(),
                    p,
                ))
                .child(
                    h_flex()
                        .gap(rems(space::XXS))
                        .child(if params.has_discovered_models {
                            Button::new("ai-open-model-picker")
                                .ghost()
                                .label(rust_i18n::t!("ai.picker_select_models").to_string())
                                .on_click(move |_, w, cx| (actions.on_open_model_picker)(w, cx))
                        } else {
                            Button::new("ai-fetch-models")
                                .ghost()
                                .label(rust_i18n::t!("ai.fetch_models").to_string())
                                .on_click(move |_, w, cx| (actions.on_fetch_models)(w, cx))
                        })
                        .child(
                            Button::new("ai-add-model-row")
                                .ghost()
                                .label(rust_i18n::t!("ai.cfg_add_manual").to_string())
                                .on_click(move |_, w, cx| (actions.on_add_model_row)(w, cx)),
                        ),
                ),
        )
        .child(params.models_section);

    if let Some(status) = params.discovery_status {
        root = root.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(status.to_owned()),
        );
    }

    if let Some(picker) = params.picker_panel {
        root = root.child(picker);
    }

    let selected_preset = acp_presets.get(params.selected_acp_idx);
    let mut root_acp = root
        .child(
            h_flex().justify_end().child(
                Button::new("ai-save-provider")
                    .primary()
                    .label(rust_i18n::t!("ai.cfg_save").to_string())
                    .on_click(move |_, w, cx| (actions.on_save_provider)(w, cx)),
            ),
        )
        .child(field_label(
            rust_i18n::t!("ai.configured_providers").to_string(),
            p,
        ))
        .child(providers_box)
        .child(
            div()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
                .child(rust_i18n::t!("ai.acp_section_title").to_string()),
        )
        .child(acp_row)
        .child(field_label(
            rust_i18n::t!("ai.acp_cmd_label").to_string(),
            p,
        ))
        .child(Input::new(params.acp_cmd_input));

    if let Some(preset) = selected_preset
        && !preset.install_command.is_empty()
    {
        root_acp = root_acp.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child(
                    rust_i18n::t!("ai.acp_install_cmd_label", cmd = preset.install_command)
                        .to_string(),
                ),
        );
    }

    if let Some((_, _, detail)) = params.acp_install_status {
        root_acp = root_acp.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.primary)
                .child(detail.clone()),
        );
    }

    let installing = params.acp_install_status.is_some_and(|(_, r, _)| *r);
    let install_label = if installing {
        rust_i18n::t!("ai.acp_installing").to_string()
    } else {
        rust_i18n::t!("ai.acp_install_btn").to_string()
    };

    root_acp
        .child(
            h_flex()
                .justify_end()
                .gap(rems(space::XS))
                .child(
                    Button::new("ai-install-acp")
                        .ghost()
                        .label(install_label)
                        .on_click(move |_, w, cx| (actions.on_install_acp)(w, cx)),
                )
                .child(
                    Button::new("ai-save-acp")
                        .primary()
                        .label(rust_i18n::t!("ai.acp_save").to_string())
                        .on_click(move |_, w, cx| (actions.on_save_acp)(w, cx)),
                ),
        )
        .into_any_element()
}

/// 渲染三层记忆查看与编辑抽屉。
pub(super) fn render_memory_drawer(
    has_host: bool,
    host_name: &str,
    global_input: &Entity<InputState>,
    host_input: &Entity<InputState>,
    memory: &MemorySnapshot,
    p: &Palette,
    on_save_memory: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let _ = memory;
    let mut box_el = v_flex()
        .flex_1()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::SM))
        .overflow_y_scrollbar()
        .child(
            div()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
                .child(rust_i18n::t!("ai.global_memory_title").to_string()),
        )
        .child(Input::new(global_input));

    if has_host {
        box_el = box_el
            .child(
                div()
                    .text_size(rems(typography::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("ai.host_memory_title", host = host_name).to_string()),
            )
            .child(Input::new(host_input));
    }

    box_el = box_el.child(
        h_flex().justify_end().child(
            Button::new("ai-save-memory")
                .primary()
                .label(rust_i18n::t!("ai.save_memory").to_string())
                .on_click(move |_, w, cx| on_save_memory(w, cx)),
        ),
    );

    box_el.into_any_element()
}

impl super::view::AiAssistantView {
    pub(super) fn render_memory_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let has_host = self.state.active_session.is_some();
        let general = rust_i18n::t!("ai.general_label").to_string();
        let host_name = self
            .state
            .active_session
            .as_ref()
            .map(|s| s.host_name.as_str())
            .unwrap_or(&general);
        let view = cx.entity();
        render_memory_drawer(
            has_host,
            host_name,
            &self.global_mem_input,
            &self.host_mem_input,
            &self.state.memory,
            p,
            move |w, cx| {
                view.update(cx, |this, cx| this.save_memory_from_drawer(w, cx));
            },
        )
    }
}
