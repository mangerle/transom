//! AI 面板内嵌的设置抽屉（分段导航 + 卡片化布局）与三层记忆编辑抽屉。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::*;
use transom_ai::{
    AiSettings, MemorySnapshot, ProviderProtocol, ThinkingEffort, default_acp_presets,
    is_acp_command_available,
};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::state::AiConfigSection;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 下拉选择器选项。
#[derive(Clone)]
pub(super) struct ConfigChoice {
    value: String,
    title: SharedString,
}

impl ConfigChoice {
    pub(super) fn new(value: &str, title: String) -> Self {
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

pub(super) type ConfigSelect = SelectState<SearchableVec<ConfigChoice>>;

/// 模型配置表中的单行草稿状态。
pub(super) struct DraftModelRow {
    pub(super) display_name: Entity<InputState>,
    pub(super) request_model: Entity<InputState>,
    pub(super) context_window: Entity<InputState>,
    pub(super) effort: Option<ThinkingEffort>,
}

/// 供应商配置抽屉渲染参数。
pub(super) struct ConfigDrawerParams<'a> {
    pub(super) active_section: AiConfigSection,
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
    pub(super) lang_select: &'a Entity<ConfigSelect>,
    pub(super) running_behavior_select: &'a Entity<ConfigSelect>,
    pub(super) policy_hooks_section: AnyElement,
    pub(super) p: &'a Palette,
}

pub(super) type ActionCb = Box<dyn Fn(&mut Window, &mut App)>;
pub(super) type IndexCb = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub(super) type ProtocolCb = Rc<dyn Fn(ProviderProtocol, &mut Window, &mut App)>;
pub(super) type ProviderIdCb = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub(super) type SectionCb = Rc<dyn Fn(AiConfigSection, &mut Window, &mut App)>;

/// 供应商配置抽屉交互回调。
pub(super) struct ConfigDrawerActions {
    pub(super) on_select_section: SectionCb,
    pub(super) on_select_protocol: ProtocolCb,
    pub(super) on_test_connection: ActionCb,
    pub(super) on_fetch_models: ActionCb,
    pub(super) on_open_model_picker: ActionCb,
    pub(super) on_add_model_row: ActionCb,
    pub(super) on_save_provider: ActionCb,
    pub(super) on_edit_provider: ProviderIdCb,
    pub(super) on_delete_provider: ProviderIdCb,
    pub(super) on_apply_acp_preset: IndexCb,
    pub(super) on_save_acp: ActionCb,
    pub(super) on_install_acp: ProviderIdCb,
}

pub(super) fn field_label(text: String, p: &Palette) -> Div {
    div()
        .text_size(rems(typography::CAPTION))
        .text_color(p.text.muted)
        .child(text)
}

pub(super) fn field_group(label: String, control: impl IntoElement, p: &Palette) -> Div {
    v_flex()
        .w_full()
        .gap(px(4.0))
        .child(field_label(label, p))
        .child(control)
}

/// 统一设置卡片外壳：圆角卡片 + 清晰标题行。
pub(super) fn section_card(title: String, header_right: Option<AnyElement>, p: &Palette) -> Div {
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
        (AiConfigSection::Models, "模型与供应商"),
        (AiConfigSection::General, "通用与 Agent"),
        (AiConfigSection::PolicyHooks, "策略与钩子"),
    ];
    let mut bar = h_flex().w_full().p(px(3.0)).gap(px(3.0)).rounded(p.radius.md).bg(p.secondary);
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
            tab.bg(p.surface.floating).shadow_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.foreground)
        } else {
            tab.text_color(p.text.muted).hover(|s| s.text_color(p.foreground))
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
pub(super) fn render_config_drawer(
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

    v_flex().flex_1().w_full().min_h_0().overflow_hidden().child(tabs).child(scroll_body).into_any_element()
}

fn render_models_tab(params: ConfigDrawerParams<'_>, actions: ConfigDrawerActions) -> Vec<AnyElement> {
    let p = params.p;
    let mut proto_row = h_flex().w_full().p(px(3.0)).gap(px(3.0)).rounded(p.radius.md).bg(p.secondary).flex_wrap();
    for proto in ProviderProtocol::ALL {
        let cb = actions.on_select_protocol.clone();
        let btn = Button::new(SharedString::from(format!("ai-proto-{}", proto.as_str())))
            .xsmall()
            .label(proto.label());
        let btn = if proto == params.selected_protocol { btn.primary() } else { btn.ghost() };
        proto_row = proto_row.child(btn.on_click(move |_, w, cx| cb(proto, w, cx)));
    }

    let url_header = h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(field_label(rust_i18n::t!("ai.cfg_api_url_label").to_string(), p))
        .child(
            Button::new("ai-test-conn").ghost().xsmall().label(rust_i18n::t!("ai.test_connection").to_string())
                .on_click(move |_, w, cx| (actions.on_test_connection)(w, cx)),
        );

    let models_header = h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(field_label(rust_i18n::t!("ai.cfg_models_title").to_string(), p))
        .child(
            h_flex()
                .gap(rems(space::XXS))
                .child(if params.has_discovered_models {
                    Button::new("ai-open-model-picker").ghost().xsmall()
                        .label(rust_i18n::t!("ai.picker_select_models").to_string())
                        .on_click(move |_, w, cx| (actions.on_open_model_picker)(w, cx))
                } else {
                    Button::new("ai-fetch-models").ghost().xsmall()
                        .label(rust_i18n::t!("ai.fetch_models").to_string())
                        .on_click(move |_, w, cx| (actions.on_fetch_models)(w, cx))
                })
                .child(
                    Button::new("ai-add-model-row").ghost().xsmall()
                        .label(rust_i18n::t!("ai.cfg_add_manual").to_string())
                        .on_click(move |_, w, cx| (actions.on_add_model_row)(w, cx)),
                ),
        );

    let mut form_card = section_card("供应商配置".to_owned(), None, p)
        .child(field_group(rust_i18n::t!("ai.cfg_name_label").to_string(), Input::new(params.provider_name_input), p))
        .child(field_group(rust_i18n::t!("ai.api_key_label").to_string(), Input::new(params.api_key_input), p))
        .child(v_flex().w_full().gap(px(4.0)).child(url_header).child(Input::new(params.base_url_input)))
        .child(field_group(rust_i18n::t!("ai.cfg_protocol_label").to_string(), proto_row, p))
        .child(
            v_flex().w_full().gap(px(6.0)).child(models_header).child(
                div().w_full().p(rems(space::XS)).rounded(p.radius.sm).bg(p.secondary).child(params.models_section),
            ),
        );

    if let Some(status) = params.discovery_status {
        let is_err = status.contains("失败") || status.contains("401") || status.contains("403") || status.contains("error") || status.contains("Error");
        form_card = form_card.child(
            div()
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(if is_err { p.danger.opacity(0.08) } else { p.secondary })
                .border_1()
                .border_color(if is_err { p.danger.opacity(0.45) } else { p.divider(p.border) })
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
            Button::new("ai-save-provider").primary().label(rust_i18n::t!("ai.cfg_save").to_string())
                .on_click(move |_, w, cx| (actions.on_save_provider)(w, cx)),
        ),
    );

    let mut cards = vec![form_card.into_any_element()];
    if !params.settings.providers.is_empty() {
        let mut list_card = section_card(rust_i18n::t!("ai.configured_providers").to_string(), None, p);
        for (idx, prov) in params.settings.providers.iter().enumerate() {
            let models: Vec<&str> = prov.models.iter().map(|m| m.display_label()).collect();
            let (edit_cb, del_cb) = (actions.on_edit_provider.clone(), actions.on_delete_provider.clone());
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
                        h_flex().flex_1().min_w_0().gap(px(6.0)).items_center().overflow_hidden()
                            .child(div().flex_shrink_0().text_size(rems(typography::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(p.foreground).child(prov.name.clone()))
                            .child(div().flex_shrink_0().px(px(6.0)).py(px(1.0)).rounded(p.radius.sm).bg(p.surface.floating).text_size(rems(typography::CAPTION)).text_color(p.text.muted).child(prov.protocol.label()))
                            .child(div().flex_1().min_w_0().truncate().text_size(rems(typography::CAPTION)).text_color(p.text.muted).child(models.join(", "))),
                    )
                    .child(
                        Button::new(("ai-del-prov", idx)).ghost().xsmall().accessibility_label(del_tip.clone()).tooltip(del_tip)
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

fn render_preference_row(label: String, select: &Entity<ConfigSelect>, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .py(px(2.0))
        .gap(rems(space::SM))
        .justify_between()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.foreground)
                .child(label),
        )
        .child(
            div()
                .w(px(148.0))
                .flex_shrink_0()
                .child(Select::new(select).small().w_full()),
        )
        .into_any_element()
}

fn render_general_acp_tab(params: ConfigDrawerParams<'_>, actions: ConfigDrawerActions) -> Vec<AnyElement> {
    let p = params.p;
    let pref_card = section_card("交互与偏好".to_owned(), None, p)
        .child(render_preference_row(rust_i18n::t!("ai.cfg_language_label").to_string(), params.lang_select, p))
        .child(div().w_full().border_t_1().border_color(p.divider(p.border)).pt(rems(space::XS)).child(
            render_preference_row(rust_i18n::t!("ai.running_msg_title").to_string(), params.running_behavior_select, p),
        ));

    let acp_presets = default_acp_presets();
    let mut acp_card = section_card(rust_i18n::t!("ai.acp_section_title").to_string(), None, p);
    for (idx, preset) in acp_presets.iter().enumerate() {
        let selected = idx == params.selected_acp_idx;
        let saved_cmd = params.settings.acp_agents.iter().find(|a| a.id == preset.id).map(|a| a.command.as_str());
        let effective_cmd = saved_cmd.unwrap_or(&preset.command);
        let installed = is_acp_command_available(effective_cmd);
        let installing = params.acp_install_status.as_ref().is_some_and(|(id, run, _)| id == &preset.id && *run);
        let cb_sel = actions.on_apply_acp_preset.clone();
        let cb_inst = actions.on_install_acp.clone();
        let pid = preset.id.clone();

        let status_el = if installed {
            h_flex()
                .flex_shrink_0()
                .px(px(8.0))
                .py(px(3.0))
                .gap(px(4.0))
                .items_center()
                .rounded(p.radius.sm)
                .bg(p.success.opacity(0.12))
                .child(div().text_color(p.success).child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small()))
                .child(div().text_size(rems(typography::CAPTION)).font_weight(FontWeight::MEDIUM).text_color(p.success).child("已安装"))
                .into_any_element()
        } else {
            let lbl = if installing { rust_i18n::t!("ai.acp_installing").to_string() } else { "下载安装".to_owned() };
            Button::new(("ai-acp-install", idx))
                .outline()
                .xsmall()
                .child(
                    h_flex().gap(px(4.0)).items_center()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                        .child(lbl),
                )
                .on_click(move |_, w, cx| {
                    cx.stop_propagation();
                    cb_inst(pid.clone(), w, cx);
                })
                .into_any_element()
        };

        let mut row = h_flex()
            .id(("ai-acp-row", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .justify_between()
            .items_center()
            .rounded(p.radius.sm)
            .border_1()
            .cursor_pointer()
            .on_click(move |_, w, cx| cb_sel(idx, w, cx))
            .child(
                h_flex().flex_1().min_w_0().gap(rems(space::XS)).items_center().overflow_hidden()
                    .child(div().flex_shrink_0().text_color(if selected { p.primary } else { p.text.muted }).child(AppIconElement::new(super::popups::backend_icon(Some(&preset.id))).small()))
                    .child(
                        v_flex().flex_1().min_w_0().gap(px(1.0)).overflow_hidden()
                            .child(div().truncate().text_size(rems(typography::CAPTION)).font_weight(FontWeight::SEMIBOLD).text_color(p.foreground).child(preset.name.clone()))
                            .child(div().truncate().font_family("monospace").text_size(rems(typography::CAPTION)).text_color(p.text.muted).child(if installed { format!("$ {effective_cmd}") } else { preset.install_command.clone() })),
                    ),
            )
            .child(status_el);
        row = if selected {
            row.bg(p.primary.opacity(0.08)).border_color(p.primary.opacity(0.45))
        } else {
            row.bg(p.secondary).border_color(p.divider(p.border)).hover(|s| s.bg(p.surface.hover))
        };
        acp_card = acp_card.child(row);
    }

    acp_card = acp_card.child(
        v_flex()
            .w_full()
            .pt(rems(space::XS))
            .gap(px(4.0))
            .border_t_1()
            .border_color(p.divider(p.border))
            .child(field_label(rust_i18n::t!("ai.acp_cmd_label").to_string(), p))
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(div().flex_1().min_w_0().child(Input::new(params.acp_cmd_input).small()))
                    .child(
                        Button::new("ai-save-acp")
                            .primary()
                            .small()
                            .label(rust_i18n::t!("ai.acp_save").to_string())
                            .on_click(move |_, w, cx| (actions.on_save_acp)(w, cx)),
                    ),
            ),
    );
    if let Some((_, _, detail)) = params.acp_install_status {
        acp_card = acp_card.child(div().text_size(rems(typography::CAPTION)).text_color(p.primary).child(detail.clone()));
    }

    vec![pref_card.into_any_element(), acp_card.into_any_element()]
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
    let mut box_el = v_flex().flex_1().w_full().p(rems(space::SM)).gap(rems(space::SM)).overflow_y_scrollbar()
        .child(div().text_size(rems(typography::BODY)).font_weight(FontWeight::SEMIBOLD).text_color(p.foreground).child(rust_i18n::t!("ai.global_memory_title").to_string()))
        .child(Input::new(global_input));
    if has_host {
        box_el = box_el
            .child(div().text_size(rems(typography::BODY)).font_weight(FontWeight::SEMIBOLD).text_color(p.foreground).child(rust_i18n::t!("ai.host_memory_title", host = host_name).to_string()))
            .child(Input::new(host_input));
    }
    box_el = box_el.child(
        h_flex().justify_end().child(Button::new("ai-save-memory").primary().label(rust_i18n::t!("ai.save_memory").to_string()).on_click(move |_, w, cx| on_save_memory(w, cx))),
    );
    box_el.into_any_element()
}

impl super::view::AiAssistantView {
    pub(super) fn render_memory_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let has_host = self.state.active_session.is_some();
        let general = rust_i18n::t!("ai.general_label").to_string();
        let host_name = self.state.active_session.as_ref().map(|s| s.host_name.as_str()).unwrap_or(&general);
        let view = cx.entity();
        render_memory_drawer(
            has_host,
            host_name,
            &self.global_mem_input,
            &self.host_mem_input,
            &self.state.memory,
            p,
            move |w, cx| view.update(cx, |this, cx| this.save_memory_from_drawer(w, cx)),
        )
    }
}
