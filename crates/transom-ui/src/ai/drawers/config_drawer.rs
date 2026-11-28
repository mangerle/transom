//! AI 面板内嵌的设置抽屉（分段导航 + 卡片化布局）与三层记忆编辑抽屉。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::SelectState;
use gpui_kit::*;
use transom_ai_contracts::{AiSettings, ProviderProtocol, ThinkingEffort};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::app::ai::config_drawer_general::render_general_acp_tab;

use crate::app::ai::state::AiConfigSection;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 下拉选择器选项。
#[derive(Clone)]
pub(in crate::app::ai) struct ConfigChoice {
    value: String,
    title: SharedString,
}

impl ConfigChoice {
    pub(in crate::app::ai) fn new(value: &str, title: String) -> Self {
        Self {
            value: value.to_owned(),
            title: title.into(),
        }
    }
}

impl SearchableListItem for ConfigChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

pub(in crate::app::ai) type ConfigSelect = SelectState<SearchableVec<ConfigChoice>>;

/// 模型配置表中的单行草稿状态。
pub(in crate::app::ai) struct DraftModelRow {
    pub(in crate::app::ai) display_name: Entity<InputState>,
    pub(in crate::app::ai) request_model: Entity<InputState>,
    pub(in crate::app::ai) context_window: Entity<InputState>,
    pub(in crate::app::ai) effort: Option<ThinkingEffort>,
}

/// 供应商配置抽屉渲染参数。
pub(in crate::app::ai) struct ConfigDrawerParams<'a> {
    pub(in crate::app::ai) active_section: AiConfigSection,
    pub(in crate::app::ai) settings: &'a AiSettings,
    pub(in crate::app::ai) selected_protocol: ProviderProtocol,
    pub(in crate::app::ai) provider_name_input: &'a Entity<InputState>,
    pub(in crate::app::ai) api_key_input: &'a Entity<InputState>,
    pub(in crate::app::ai) base_url_input: &'a Entity<InputState>,
    pub(in crate::app::ai) models_section: AnyElement,
    pub(in crate::app::ai) picker_panel: Option<AnyElement>,
    pub(in crate::app::ai) has_discovered_models: bool,
    pub(in crate::app::ai) acp_cmd_input: &'a Entity<InputState>,
    pub(in crate::app::ai) selected_acp_idx: usize,
    pub(in crate::app::ai) acp_install_status: Option<&'a (String, bool, String)>,
    pub(in crate::app::ai) discovery_status: Option<&'a str>,
    pub(in crate::app::ai) lang_select: &'a Entity<ConfigSelect>,
    pub(in crate::app::ai) running_behavior_select: &'a Entity<ConfigSelect>,
    pub(in crate::app::ai) policy_hooks_section: AnyElement,
    pub(in crate::app::ai) p: &'a Palette,
}

pub(in crate::app::ai) type ActionCb = Box<dyn Fn(&mut Window, &mut App)>;
pub(in crate::app::ai) type IndexCb = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub(in crate::app::ai) type ProtocolCb = Rc<dyn Fn(ProviderProtocol, &mut Window, &mut App)>;
pub(in crate::app::ai) type ProviderIdCb = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub(in crate::app::ai) type SectionCb = Rc<dyn Fn(AiConfigSection, &mut Window, &mut App)>;

/// 供应商配置抽屉交互回调。
pub(in crate::app::ai) struct ConfigDrawerActions {
    pub(in crate::app::ai) on_select_section: SectionCb,
    pub(in crate::app::ai) on_select_protocol: ProtocolCb,
    pub(in crate::app::ai) on_test_connection: ActionCb,
    pub(in crate::app::ai) on_fetch_models: ActionCb,
    pub(in crate::app::ai) on_open_model_picker: ActionCb,
    pub(in crate::app::ai) on_add_model_row: ActionCb,
    pub(in crate::app::ai) on_save_provider: ActionCb,
    pub(in crate::app::ai) on_edit_provider: ProviderIdCb,
    pub(in crate::app::ai) on_delete_provider: ProviderIdCb,
    pub(in crate::app::ai) on_apply_acp_preset: IndexCb,
    pub(in crate::app::ai) on_save_acp: ActionCb,
    pub(in crate::app::ai) on_install_acp: ProviderIdCb,
}

pub(in crate::app::ai) fn field_label(text: String, p: &Palette) -> Div {
    div()
        .text_size(rems(typography::CAPTION))
        .text_color(p.text.muted)
        .child(text)
}

pub(in crate::app::ai) fn field_group(
    label: String,
    control: impl IntoElement,
    p: &Palette,
) -> Div {
    v_flex()
        .w_full()
        .gap(px(4.0))
        .child(field_label(label, p))
        .child(control)
}

/// 统一设置卡片外壳：圆角卡片 + 清晰标题行。
pub(in crate::app::ai) fn section_card(
    title: String,
    header_right: Option<AnyElement>,
    p: &Palette,
) -> Div {
    let mut header = h_flex().w_full().justify_between().items_center().child(
        div()
            .text_size(rems(typography::BODY))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.foreground)
            .child(title),
    );
    if let Some(right) = header_right {
        header = header.child(right);
    }
    v_flex()
        .w_full()
        .p(rems(space::MD))
        .gap(rems(space::SM))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.divider(p.border))
        .child(header)
}

fn render_config_tabs(active: AiConfigSection, p: &Palette, on_select: SectionCb) -> AnyElement {
    let items = [
        (
            AiConfigSection::Models,
            rust_i18n::t!("ai.cfg_tab_models").to_string(),
        ),
        (
            AiConfigSection::General,
            rust_i18n::t!("ai.cfg_tab_general").to_string(),
        ),
        (
            AiConfigSection::PolicyHooks,
            rust_i18n::t!("ai.cfg_tab_policy").to_string(),
        ),
    ];
    let mut bar = h_flex()
        .w_full()
        .p(px(3.0))
        .gap(px(3.0))
        .rounded(p.radius.md)
        .bg(p.secondary);
    for (idx, (section, label)) in items.into_iter().enumerate() {
        let cb = on_select.clone();
        let active_tab = section == active;
        let mut tab = h_flex()
            .id(("ai-cfg-tab", idx))
            .flex_1()
            .min_w_0()
            .px(rems(space::XS))
            .py(px(5.0))
            .items_center()
            .justify_center()
            .rounded(p.radius.sm)
            .cursor_pointer()
            .text_size(rems(typography::CAPTION))
            .on_click(move |_, w, cx| cb(section, w, cx))
            .child(div().truncate().child(label));
        tab = if active_tab {
            tab.bg(p.surface.floating)
                .shadow_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
        } else {
            tab.text_color(p.text.muted)
                .hover(|s| s.text_color(p.foreground))
        };
        bar = bar.child(tab);
    }
    div()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .border_b_1()
        .border_color(p.divider(p.border))
        .bg(p.background)
        .child(bar)
        .into_any_element()
}

/// 渲染协议驱动的模型配置与分栏设置抽屉。
pub(in crate::app::ai) fn render_config_drawer(
    params: ConfigDrawerParams<'_>,
    actions: ConfigDrawerActions,
) -> AnyElement {
    let p = params.p;
    let tabs = render_config_tabs(params.active_section, p, actions.on_select_section.clone());

    let mut scroll_body = v_flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .p(rems(space::SM))
        .gap(rems(space::SM))
        .overflow_y_scrollbar();

    match params.active_section {
        AiConfigSection::Models => {
            for card in render_models_tab(params, actions) {
                scroll_body = scroll_body.child(card);
            }
        }
        AiConfigSection::General => {
            for card in render_general_acp_tab(params, actions) {
                scroll_body = scroll_body.child(card);
            }
        }
        AiConfigSection::PolicyHooks => {
            scroll_body = scroll_body.child(params.policy_hooks_section);
        }
    }

    v_flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .overflow_hidden()
        .child(tabs)
        .child(scroll_body)
        .into_any_element()
}

fn render_models_tab(
    params: ConfigDrawerParams<'_>,
    actions: ConfigDrawerActions,
) -> Vec<AnyElement> {
    let p = params.p;
    let mut proto_row = h_flex()
        .w_full()
        .p(px(3.0))
        .gap(px(3.0))
        .rounded(p.radius.md)
        .bg(p.secondary)
        .flex_wrap();
    for proto in ProviderProtocol::ALL {
        let cb = actions.on_select_protocol.clone();
        let btn = Button::new(SharedString::from(format!("ai-proto-{}", proto.as_str())))
            .xsmall()
            .label(proto.label());
        let btn = if proto == params.selected_protocol {
            btn.primary()
        } else {
            btn.ghost()
        };
        proto_row = proto_row.child(btn.on_click(move |_, w, cx| cb(proto, w, cx)));
    }

    let url_header = h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(field_label(
            rust_i18n::t!("ai.cfg_api_url_label").to_string(),
            p,
        ))
        .child(
            Button::new("ai-test-conn")
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.test_connection").to_string())
                .on_click(move |_, w, cx| (actions.on_test_connection)(w, cx)),
        );

    let models_header = h_flex()
        .w_full()
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
                        .xsmall()
                        .label(rust_i18n::t!("ai.picker_select_models").to_string())
                        .on_click(move |_, w, cx| (actions.on_open_model_picker)(w, cx))
                } else {
                    Button::new("ai-fetch-models")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("ai.fetch_models").to_string())
                        .on_click(move |_, w, cx| (actions.on_fetch_models)(w, cx))
                })
                .child(
                    Button::new("ai-add-model-row")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("ai.cfg_add_manual").to_string())
                        .on_click(move |_, w, cx| (actions.on_add_model_row)(w, cx)),
                ),
        );

    let mut form_card = section_card(
        rust_i18n::t!("ai.cfg_provider_section").to_string(),
        None,
        p,
    )
    .child(field_group(
        rust_i18n::t!("ai.cfg_name_label").to_string(),
        Input::new(params.provider_name_input),
        p,
    ))
    .child(field_group(
        rust_i18n::t!("ai.api_key_label").to_string(),
        Input::new(params.api_key_input),
        p,
    ))
    .child(
        v_flex()
            .w_full()
            .gap(px(4.0))
            .child(url_header)
            .child(Input::new(params.base_url_input)),
    )
    .child(field_group(
        rust_i18n::t!("ai.cfg_protocol_label").to_string(),
        proto_row,
        p,
    ))
    .child(
        v_flex().w_full().gap(px(6.0)).child(models_header).child(
            div()
                .w_full()
                .p(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(p.secondary)
                .child(params.models_section),
        ),
    );

    if let Some(status) = params.discovery_status {
        let is_err = status.contains(rust_i18n::t!("ai.cfg_status_failed_kw").as_ref())
            || status.contains("401")
            || status.contains("403")
            || status.contains("error")
            || status.contains("Error");
        form_card = form_card.child(
            div()
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(if is_err {
                    p.danger.opacity(0.08)
                } else {
                    p.secondary
                })
                .border_1()
                .border_color(if is_err {
                    p.danger.opacity(0.45)
                } else {
                    p.divider(p.border)
                })
                .text_size(rems(typography::CAPTION))
                .text_color(if is_err { p.danger } else { p.muted_foreground })
                .child(status.to_owned()),
        );
    }
    if let Some(picker) = params.picker_panel {
        form_card = form_card.child(picker);
    }
    form_card = form_card.child(
        h_flex().w_full().justify_end().pt(px(2.0)).child(
            Button::new("ai-save-provider")
                .primary()
                .label(rust_i18n::t!("ai.cfg_save").to_string())
                .on_click(move |_, w, cx| (actions.on_save_provider)(w, cx)),
        ),
    );

    let mut cards = vec![form_card.into_any_element()];
    if !params.settings.providers.is_empty() {
        let mut list_card = section_card(
            rust_i18n::t!("ai.configured_providers").to_string(),
            None,
            p,
        );
        for (idx, prov) in params.settings.providers.iter().enumerate() {
            let models: Vec<&str> = prov.models.iter().map(|m| m.display_label()).collect();
            let (edit_cb, del_cb) = (
                actions.on_edit_provider.clone(),
                actions.on_delete_provider.clone(),
            );
            let (edit_id, del_id) = (prov.id.clone(), prov.id.clone());
            let del_tip = rust_i18n::t!("ai.delete_provider").to_string();
            list_card = list_card.child(
                h_flex()
                    .id(("ai-prov-row", idx))
                    .w_full()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(rems(space::XS))
                    .justify_between()
                    .items_center()
                    .rounded(p.radius.sm)
                    .bg(p.secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(p.surface.hover))
                    .on_click(move |_, w, cx| edit_cb(edit_id.clone(), w, cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.0))
                            .items_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(rems(typography::CAPTION))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.foreground)
                                    .child(prov.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(p.radius.sm)
                                    .bg(p.surface.floating)
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(prov.protocol.label()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(models.join(", ")),
                            ),
                    )
                    .child(
                        Button::new(("ai-del-prov", idx))
                            .ghost()
                            .xsmall()
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
        cards.push(list_card.into_any_element());
    }
    cards
}
