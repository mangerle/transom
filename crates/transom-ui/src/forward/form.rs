//! 转发面板的新增/编辑表单、预设按钮与公网确认弹窗体。

use super::ForwardKindChoice;
use super::ForwardView;
use crate::app::{AppShell, AppState};
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
    /// 转发类型三选一：本地/远程/动态。
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
        let preset_buttons = h_flex()
            .gap_1()
            .children(transom_core::FORWARD_PRESETS.iter().map(|preset| {
                Button::new(ElementId::Name(
                    format!("forward-preset-{}", preset.listen_port).into(),
                ))
                .small()
                .outline()
                .label(rust_i18n::t!(preset.label_key).to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    // 预设只填草稿字段，不直接建规则：名称留给用户改。
                    this.state.kind = match preset.kind {
                        transom_core::ForwardPresetKind::Local => ForwardKindChoice::Local,
                        transom_core::ForwardPresetKind::Dynamic => ForwardKindChoice::Dynamic,
                    };
                    this.state.bind_port_input.update(cx, |input, cx| {
                        input.set_value(preset.listen_port.to_string(), window, cx)
                    });
                    if preset.needs_target() {
                        this.state.target_port_input.update(cx, |input, cx| {
                            input.set_value(preset.target_port.to_string(), window, cx)
                        });
                    }
                    cx.notify();
                }))
            }));
        let kind_help = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(match kind {
                ForwardKindChoice::Local => rust_i18n::t!("forward.kind_help_local").to_string(),
                ForwardKindChoice::Remote => rust_i18n::t!("forward.kind_help_remote").to_string(),
                ForwardKindChoice::Dynamic => {
                    rust_i18n::t!("forward.kind_help_dynamic").to_string()
                }
            });

        let mut add_form = v_flex()
            .p_2()
            .gap_2()
            .border_color(p.divider(p.sidebar_border))
            .divider_edge(DividerEdge::Top, p.divider_width)
            .child(kind_buttons)
            .child(kind_help)
            .child(preset_buttons)
            .child(Input::new(&name_input).w_full())
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().child(Input::new(&bind_host_input).w_full()))
                    .child(
                        div()
                            .w(px(90.0))
                            .child(Input::new(&bind_port_input).w_full()),
                    ),
            );
        if kind != ForwardKindChoice::Dynamic {
            add_form = add_form.child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&target_host_input).w_full()),
                    )
                    .child(
                        div()
                            .w(px(90.0))
                            .child(Input::new(&target_port_input).w_full()),
                    ),
            );
        }
        // 编辑态：保存修改 + 取消；新增态：单一添加按钮。
        let actions = if editing {
            h_flex()
                .gap_2()
                .child(
                    Button::new("forward-save-edit")
                        .primary()
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small())
                        .label(rust_i18n::t!("forward.save_edit").to_string())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(rule) = this.build_rule(cx) {
                                cx.emit(super::ForwardPanelEvent::UpdateRule { tab_id, rule });
                                this.cancel_edit(window, cx);
                                cx.notify();
                            }
                        })),
                )
                .child(crate::widgets::dialog_cancel_button(
                    "forward-cancel-edit",
                    rust_i18n::t!("common.cancel").to_string(),
                    cx.listener(move |this, _, window, cx| {
                        this.cancel_edit(window, cx);
                    }),
                ))
        } else {
            h_flex().child(
                Button::new("forward-add")
                    .primary()
                    .small()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                    .label(rust_i18n::t!("forward.add").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // 表单校验在实体里完成，通过后把规则交给外壳落库并发命令。
                        if let Some(rule) = this.build_rule(cx) {
                            cx.emit(super::ForwardPanelEvent::SubmitRule { tab_id, rule });
                            cx.notify();
                        }
                    })),
            )
        };
        add_form = add_form.child(actions);
        if let Some(error) = panel_error {
            add_form = add_form.child(div().text_xs().text_color(p.danger).child(error));
        }

        add_form.into_any_element()
    }
}

/// 端口转发冲突确认的弹窗体。
///
/// 取消与"确认启动"分别绑到 `cancel_forward_confirm` /
/// `confirm_forward_start`，Esc 走前者：把拒绝回给等待中的转发请求。
pub(crate) fn render_forward_confirm_body(
    app_state: &AppState,
    app: &Entity<AppShell>,
    _cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(pending) = app_state.ui.modals.tools.confirm_forward_any.as_ref() else {
        return div().into_any_element();
    };
    let rule_name = pending.rule.name.clone();

    let content = v_flex().gap_3().child(
        div()
            .text_xs()
            .px_2()
            .py_1()
            .rounded(p.radius.sm)
            .bg(p.muted)
            .text_color(p.muted_foreground)
            .child(format!("Rule: {rule_name}")),
    );

    let footer = h_flex()
        .gap_2()
        .child(crate::widgets::dialog_cancel_button(
            "cancel-forward-confirm",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                crate::forward::cancel_forward_confirm(&mut this.state, cx)
            }),
        ))
        .child(crate::widgets::dialog_confirm_button(
            "accept-forward-confirm",
            rust_i18n::t!("forward.confirm_start").to_string(),
            None,
            crate::widgets::DialogConfirmKind::Danger,
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                crate::forward::confirm_forward_start(&mut this.state, cx);
            }),
        ));

    crate::widgets::dialog_body(
        "forward-confirm-modal",
        rust_i18n::t!("forward.confirm_any_title").to_string(),
        Some(rust_i18n::t!("forward.confirm_any_hint").to_string()),
        content,
        Some(footer),
    )
}
