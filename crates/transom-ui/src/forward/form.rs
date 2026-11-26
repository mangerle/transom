//! 转发面板的新增/编辑表单与预设按钮。

use super::ForwardKindChoice;
use super::ForwardView;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl ForwardView {
    /// 应用一条端口转发预设，只填充空名称与端口字段。
    fn apply_preset(
        &mut self,
        preset: &'static transom_core::ForwardPreset,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.kind = match preset.kind {
            transom_core::ForwardPresetKind::Local => ForwardKindChoice::Local,
            transom_core::ForwardPresetKind::Dynamic => ForwardKindChoice::Dynamic,
        };
        if self.state.name_input.read(cx).value().trim().is_empty() {
            self.state.name_input.update(cx, |input, cx| {
                input.set_value(label.to_string(), window, cx);
            });
        }
        self.state.bind_port_input.update(cx, |input, cx| {
            input.set_value(preset.listen_port.to_string(), window, cx);
        });
        if preset.needs_target() {
            self.state.target_port_input.update(cx, |input, cx| {
                input.set_value(preset.target_port.to_string(), window, cx);
            });
        }
        cx.notify();
    }

    /// 转发类型三选一：本地 / 远程 / 动态。
    pub(crate) fn render_kind_selector(
        &self,
        kind: ForwardKindChoice,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        ButtonGroup::new("forward-kind")
            .small()
            .child(
                Button::new("forward-mode-local")
                    .small()
                    .label(rust_i18n::t!("forward.local").to_string())
                    .selected(kind == ForwardKindChoice::Local),
            )
            .child(
                Button::new("forward-mode-remote")
                    .small()
                    .label(rust_i18n::t!("forward.remote").to_string())
                    .selected(kind == ForwardKindChoice::Remote),
            )
            .child(
                Button::new("forward-mode-dynamic")
                    .small()
                    .label(rust_i18n::t!("forward.dynamic").to_string())
                    .selected(kind == ForwardKindChoice::Dynamic),
            )
            .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                let kind = match clicks.first() {
                    Some(0) => Some(ForwardKindChoice::Local),
                    Some(1) => Some(ForwardKindChoice::Remote),
                    Some(2) => Some(ForwardKindChoice::Dynamic),
                    _ => None,
                };
                if let Some(kind) = kind {
                    this.state.kind = kind;
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    /// 端口转发规则表单渲染：编辑态复用同一份草稿，预设按钮一键填字段。
    pub(crate) fn render_forward_add_form(
        &self,
        tab_id: transom_core::TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let kind = self.state.kind;
        let editing = self.state.editing_id.is_some();
        let panel_error = self.state.error.clone();
        let name_input = self.state.name_input.clone();
        let bind_host_input = self.state.bind_host_input.clone();
        let bind_port_input = self.state.bind_port_input.clone();
        let target_host_input = self.state.target_host_input.clone();
        let target_port_input = self.state.target_port_input.clone();

        let kind_buttons = self.render_kind_selector(kind, cx);

        let preset_buttons = render_forward_presets(cx, p);
        let flow_banner = render_flow_banner(kind, p);

        let mut add_form = v_flex()
            .w_full()
            .p_2p5()
            .gap_2p5()
            .border_color(p.divider(p.sidebar_border))
            .divider_edge(DividerEdge::Top, p.divider_width);

        if editing {
            add_form = add_form.child(render_editing_badge(p));
        }

        add_form = add_form
            .child(kind_buttons)
            .child(flow_banner)
            .child(preset_buttons)
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("forward.rule_name_optional").to_string()),
                    )
                    .child(Input::new(&name_input).w_full()),
            );

        // 监听端点与目标端点字段
        let (listen_title, target_title) = forward_endpoint_titles(kind);

        add_form = add_form.child(render_endpoint_group(
            listen_title,
            rust_i18n::t!("forward.bind_host").to_string(),
            Input::new(&bind_host_input).w_full().into_any_element(),
            rust_i18n::t!("forward.bind_port").to_string(),
            Input::new(&bind_port_input).w_full().into_any_element(),
            p,
        ));

        let forward_divider = h_flex()
            .py_0p5()
            .items_center()
            .gap_2()
            .child(div().flex_1().h(px(1.0)).bg(p.stroke.faint))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(
                        AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN))
                            .xsmall()
                            .text_color(p.muted_foreground),
                    )
                    .child(rust_i18n::t!("forward.forward_to").to_string()),
            )
            .child(div().flex_1().h(px(1.0)).bg(p.stroke.faint));

        let (target_title_text, target_host_el, target_port_el) = if kind
            == ForwardKindChoice::Dynamic
        {
            (
                rust_i18n::t!("forward.target_dynamic_title").to_string(),
                render_disabled_field(rust_i18n::t!("forward.target_dynamic_hint").to_string(), p),
                render_disabled_field(rust_i18n::t!("forward.target_dynamic_port").to_string(), p),
            )
        } else {
            (
                target_title,
                Input::new(&target_host_input).w_full().into_any_element(),
                Input::new(&target_port_input).w_full().into_any_element(),
            )
        };

        add_form = add_form.child(forward_divider).child(render_endpoint_group(
            target_title_text,
            rust_i18n::t!("forward.target_host").to_string(),
            target_host_el,
            rust_i18n::t!("forward.target_port").to_string(),
            target_port_el,
            p,
        ));

        let actions = render_forward_actions(tab_id, editing, cx, p);
        add_form = add_form.child(actions);
        if let Some(error) = panel_error {
            add_form = add_form.child(render_forward_error(error, p));
        }

        add_form.into_any_element()
    }
}

fn render_forward_presets(cx: &mut Context<ForwardView>, p: &Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("forward.preset_header").to_string()),
        )
        .child(
            h_flex()
                .gap_1p5()
                .flex_wrap()
                .children(transom_core::FORWARD_PRESETS.iter().map(|preset| {
                    let preset_label = rust_i18n::t!(preset.label_key).to_string();
                    let label_for_click = preset_label.clone();
                    div()
                        .id(ElementId::Name(
                            format!("forward-preset-{}", preset.listen_port).into(),
                        ))
                        .px_2()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.surface.ghost)
                        .border_1()
                        .border_color(p.stroke.subtle)
                        .text_xs()
                        .cursor_pointer()
                        .hover(|style| style.bg(p.surface.hover).border_color(p.primary))
                        .child(preset_label)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.apply_preset(preset, &label_for_click, window, cx);
                        }))
                })),
        )
        .into_any_element()
}

fn render_flow_banner(kind: ForwardKindChoice, p: &Palette) -> AnyElement {
    let (kind_help_text, flow_text) = match kind {
        ForwardKindChoice::Local => (
            rust_i18n::t!("forward.kind_help_local").to_string(),
            rust_i18n::t!("forward.flow_local").to_string(),
        ),
        ForwardKindChoice::Remote => (
            rust_i18n::t!("forward.kind_help_remote").to_string(),
            rust_i18n::t!("forward.flow_remote").to_string(),
        ),
        ForwardKindChoice::Dynamic => (
            rust_i18n::t!("forward.kind_help_dynamic").to_string(),
            rust_i18n::t!("forward.flow_dynamic").to_string(),
        ),
    };
    v_flex()
        .min_h(px(52.0))
        .p_2()
        .gap_1()
        .rounded(p.radius.sm)
        .bg(p.surface.subtle)
        .border_1()
        .border_color(p.stroke.subtle)
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .line_height(rems(1.2))
                .child(kind_help_text),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .text_xs()
                .text_color(p.primary)
                .font_weight(FontWeight::MEDIUM)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::LIGHTNING)).xsmall())
                .child(flow_text),
        )
        .into_any_element()
}

fn render_editing_badge(p: &Palette) -> AnyElement {
    h_flex()
        .items_center()
        .gap_1p5()
        .px_2()
        .py_1()
        .rounded(p.radius.sm)
        .bg(p.tone.warning_subtle)
        .border_1()
        .border_color(p.warning.opacity(0.4))
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::EDIT))
                .small()
                .text_color(p.warning),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.warning)
                .child(rust_i18n::t!("forward.editing_badge").to_string()),
        )
        .into_any_element()
}

fn forward_endpoint_titles(kind: ForwardKindChoice) -> (String, String) {
    match kind {
        ForwardKindChoice::Local => (
            rust_i18n::t!("forward.listen_local_title").to_string(),
            rust_i18n::t!("forward.target_remote_title").to_string(),
        ),
        ForwardKindChoice::Remote => (
            rust_i18n::t!("forward.listen_remote_title").to_string(),
            rust_i18n::t!("forward.target_local_title").to_string(),
        ),
        ForwardKindChoice::Dynamic => (
            rust_i18n::t!("forward.socks5_listen_title").to_string(),
            String::new(),
        ),
    }
}

fn render_forward_actions(
    tab_id: transom_core::TabId,
    editing: bool,
    cx: &mut Context<ForwardView>,
    _p: &Palette,
) -> AnyElement {
    if editing {
        h_flex()
            .gap_2()
            .w_full()
            .child(
                div().flex_1().child(
                    Button::new("forward-save-edit")
                        .primary()
                        .small()
                        .w_full()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small())
                        .label(rust_i18n::t!("forward.save_edit").to_string())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(rule) = view.build_rule(cx) {
                                cx.emit(super::ForwardPanelEvent::UpdateRule { tab_id, rule });
                                view.cancel_edit(window, cx);
                                cx.notify();
                            }
                        })),
                ),
            )
            .child(div().flex_1().child(crate::widgets::dialog_cancel_button(
                "forward-cancel-edit",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(move |view, _, window, cx| {
                    view.cancel_edit(window, cx);
                }),
            )))
            .into_any_element()
    } else {
        h_flex()
            .w_full()
            .child(
                Button::new("forward-add")
                    .primary()
                    .small()
                    .w_full()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                    .label(rust_i18n::t!("forward.add").to_string())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(rule) = view.build_rule(cx) {
                            cx.emit(super::ForwardPanelEvent::SubmitRule { tab_id, rule });
                            cx.notify();
                        }
                    })),
            )
            .into_any_element()
    }
}

fn render_forward_error(error: String, p: &Palette) -> AnyElement {
    h_flex()
        .p_2()
        .gap_1p5()
        .items_center()
        .rounded(p.radius.sm)
        .bg(p.tone.danger_subtle)
        .border_1()
        .border_color(p.danger.opacity(0.4))
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::ERROR))
                .xsmall()
                .text_color(p.danger),
        )
        .child(div().text_xs().text_color(p.danger).child(error))
        .into_any_element()
}

/// 端点字段分组：组标题在顶，地址与端口并排并在各自输入框上方标出名称。
fn render_endpoint_group(
    title: String,
    host_label: String,
    host_field: AnyElement,
    port_label: String,
    port_field: AnyElement,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_1()
        .w_full()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.foreground)
                .child(title),
        )
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(host_label),
                        )
                        .child(host_field),
                )
                .child(
                    v_flex()
                        .w(px(96.0))
                        .gap_0p5()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(port_label),
                        )
                        .child(port_field),
                ),
        )
        .into_any_element()
}

/// 置灰占位字段：用于动态 SOCKS5 目标端点，保持表单结构与高度恒定。
fn render_disabled_field(placeholder: String, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .h(px(28.0))
        .px_2()
        .rounded(p.radius.sm)
        .bg(p.surface.ghost)
        .border_1()
        .border_color(p.stroke.faint)
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(placeholder),
        )
        .into_any_element()
}
