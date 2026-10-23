//! 端口转发面板与公网监听确认弹窗视图渲染。

use super::ForwardKindChoice;
use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_core::{ForwardKind, ForwardState};

impl AppShell {
    pub(crate) fn render_forward_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(tab_id) = self.active_tab else {
            return div().into_any_element();
        };
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return div().into_any_element();
        };
        let Some(panel) = tab.forwards_panel.as_ref() else {
            return div().into_any_element();
        };
        let Some(session) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == tab.session_id)
        else {
            return div().into_any_element();
        };
        let rules = session.forwards.clone();
        let statuses = tab.forward_statuses.clone();
        let kind = panel.kind;
        let panel_error = panel.error.clone();
        let name_input = panel.name_input.clone();
        let bind_host_input = panel.bind_host_input.clone();
        let bind_port_input = panel.bind_port_input.clone();
        let target_host_input = panel.target_host_input.clone();
        let target_port_input = panel.target_port_input.clone();

        let mut rule_rows: Vec<AnyElement> = Vec::new();
        if rules.is_empty() {
            rule_rows.push(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("forward.empty").to_string())
                    .into_any_element(),
            );
        }
        for rule in rules {
            let rule_id = rule.id;
            let status = statuses.get(&rule_id).cloned();
            let running = status.as_ref().is_some_and(|status| {
                matches!(
                    status.state,
                    ForwardState::Active { .. } | ForwardState::Starting
                )
            });
            let (status_text, status_color) = match &status {
                Some(status) => match &status.state {
                    ForwardState::Stopped => (
                        rust_i18n::t!("forward.stopped").to_string(),
                        p.muted_foreground,
                    ),
                    ForwardState::Starting => (
                        rust_i18n::t!("forward.starting").to_string(),
                        p.muted_foreground,
                    ),
                    ForwardState::Active { bound_port } => (
                        match bound_port {
                            Some(port) => {
                                format!("{} :{}", rust_i18n::t!("forward.active"), port)
                            }
                            None => rust_i18n::t!("forward.active").to_string(),
                        },
                        p.success,
                    ),
                    ForwardState::Failed => (rust_i18n::t!("forward.failed").to_string(), p.danger),
                    _ => (String::new(), p.muted_foreground),
                },
                None => (
                    rust_i18n::t!("forward.stopped").to_string(),
                    p.muted_foreground,
                ),
            };
            let summary = match &rule.kind {
                ForwardKind::Local { bind, target } => format!(
                    "L {}:{} -> {}:{}",
                    bind.host, bind.port, target.host, target.port
                ),
                ForwardKind::Remote { bind, target } => format!(
                    "R {}:{} -> {}:{}",
                    bind.host, bind.port, target.host, target.port
                ),
                ForwardKind::Dynamic { bind, .. } => format!("D {}:{}", bind.host, bind.port),
                _ => String::new(),
            };
            let mut row = v_flex()
                .gap_1()
                .p_2()
                .rounded_sm()
                .border_1()
                .border_color(p.sidebar_border)
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(div().text_sm().child(rule.name.clone()))
                        .child(div().text_xs().text_color(status_color).child(status_text)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(summary),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(if running {
                            Button::new(ElementId::Name(format!("forward-stop-{rule_id}").into()))
                                .small()
                                .label(rust_i18n::t!("forward.stop").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.stop_forward_rule(tab_id, rule_id, cx)
                                }))
                        } else {
                            Button::new(ElementId::Name(format!("forward-start-{rule_id}").into()))
                                .small()
                                .primary()
                                .label(rust_i18n::t!("forward.start").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.start_forward_rule(tab_id, rule_id, cx)
                                }))
                        })
                        .child(
                            Button::new(ElementId::Name(
                                format!("forward-delete-{rule_id}").into(),
                            ))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .label(rust_i18n::t!("forward.delete").to_string())
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.delete_forward_rule(tab_id, rule_id, cx),
                            )),
                        ),
                );
            if let Some(error) = status.and_then(|status| status.error) {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(p.danger)
                        .child(localized_error(&error)),
                );
            }
            rule_rows.push(row.into_any_element());
        }

        let local_button = if kind == ForwardKindChoice::Local {
            Button::new("forward-mode-local")
                .primary()
                .small()
                .label(rust_i18n::t!("forward.local").to_string())
        } else {
            Button::new("forward-mode-local")
                .small()
                .label(rust_i18n::t!("forward.local").to_string())
        }
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(tab_id) = this.active_tab
                && let Some(panel) = this.forward_panel_mut(tab_id)
            {
                panel.kind = ForwardKindChoice::Local;
            }
            cx.notify();
        }));
        let remote_button = if kind == ForwardKindChoice::Remote {
            Button::new("forward-mode-remote")
                .primary()
                .small()
                .label(rust_i18n::t!("forward.remote").to_string())
        } else {
            Button::new("forward-mode-remote")
                .small()
                .label(rust_i18n::t!("forward.remote").to_string())
        }
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(tab_id) = this.active_tab
                && let Some(panel) = this.forward_panel_mut(tab_id)
            {
                panel.kind = ForwardKindChoice::Remote;
            }
            cx.notify();
        }));
        let dynamic_button = if kind == ForwardKindChoice::Dynamic {
            Button::new("forward-mode-dynamic")
                .primary()
                .small()
                .label(rust_i18n::t!("forward.dynamic").to_string())
        } else {
            Button::new("forward-mode-dynamic")
                .small()
                .label(rust_i18n::t!("forward.dynamic").to_string())
        }
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(tab_id) = this.active_tab
                && let Some(panel) = this.forward_panel_mut(tab_id)
            {
                panel.kind = ForwardKindChoice::Dynamic;
            }
            cx.notify();
        }));

        let mut add_form = v_flex()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(p.sidebar_border)
            .child(
                h_flex()
                    .gap_2()
                    .child(local_button)
                    .child(remote_button)
                    .child(dynamic_button),
            )
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
        add_form = add_form.child(
            Button::new("forward-add")
                .primary()
                .small()
                .icon(IconName::Plus)
                .label(rust_i18n::t!("forward.add").to_string())
                .on_click(cx.listener(move |this, _, _, cx| this.add_forward_rule(tab_id, cx))),
        );
        if let Some(error) = panel_error {
            add_form = add_form.child(div().text_xs().text_color(p.danger).child(error));
        }

        v_flex()
            .w(px(360.0))
            .h_full()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .border_l_1()
            .border_color(p.sidebar_border)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(p.sidebar_border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(rust_i18n::t!("forward.title").to_string()),
                    )
                    .child(
                        Button::new("close-forward")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(tab_id) = this.active_tab
                                    && let Some(tab) =
                                        this.tabs.iter_mut().find(|tab| tab.id == tab_id)
                                {
                                    tab.forwards_panel = None;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id(ElementId::Name("forward-rules".into()))
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .gap_2()
                    .children(rule_rows),
            )
            .child(add_form)
            .into_any_element()
    }

    pub(crate) fn render_forward_confirm_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(pending) = self.confirm_forward_any.as_ref() else {
            return div().into_any_element();
        };
        let rule_name = pending.rule.name.clone();

        let content = v_flex().gap_3().child(
            div()
                .text_xs()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(p.muted)
                .text_color(p.muted_foreground)
                .child(format!("Rule: {rule_name}")),
        );

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-forward-confirm")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_forward_confirm(cx))),
            )
            .child(
                Button::new("accept-forward-confirm")
                    .danger()
                    .label(rust_i18n::t!("forward.confirm_start").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_forward_start(cx))),
            );

        let dialog = Self::dialog_frame(
            "forward-confirm-modal",
            rust_i18n::t!("forward.confirm_any_title").to_string(),
            Some(rust_i18n::t!("forward.confirm_any_hint").to_string()),
            content,
            Some(footer),
            px(460.0),
            p,
        );

        Self::overlay(dialog, p)
    }
}
