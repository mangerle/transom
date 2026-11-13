//! 会话表单中的“转发”分组：管理 `Session::forwards` 端口转发规则。

use super::*;
use crate::session_form::ssh::number_form_field;
use crate::session_form::types::{ConnectionChoice, SessionForwardKind};
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::switch::Switch;
use transom_core::{BindAddress, ForwardId, ForwardKind, ForwardRule, SocksOptions, TargetAddress};

impl SessionFormView {
    /// 渲染“转发”分组内容；非 SSH 连接显示仅支持 SSH 的提示。
    pub(super) fn render_session_forward_section(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if form.connection != ConnectionChoice::Ssh {
            return div()
                .p_3()
                .rounded(p.radius.md)
                .bg(p.muted)
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.forward_ssh_only").to_string())
                .into_any_element();
        }

        v_flex()
            .gap_3()
            .child(self.render_forward_rules_list(&form.forwards, p, cx))
            .child(self.render_forward_draft_form(form, p, cx))
            .into_any_element()
    }

    /// 渲染当前会话已配置的转发规则列表。
    fn render_forward_rules_list(
        &self,
        rules: &[ForwardRule],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if rules.is_empty() {
            return div()
                .p_3()
                .rounded(p.radius.md)
                .bg(p.muted)
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("forward.empty").to_string())
                .into_any_element();
        }

        let mut list = v_flex().gap_1p5();
        for rule in rules {
            let id = rule.id;
            let enabled = rule.enabled;
            let summary = summarize_forward_rule(rule);
            let row = h_flex()
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .rounded(p.radius.md)
                .border_1()
                .border_color(p.border)
                .bg(p.surface.subtle)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_sm().truncate().child(rule.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .truncate()
                                .child(summary),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("form.forward_auto_start").to_string()),
                        )
                        .child(
                            Switch::new(ElementId::Name(
                                format!("session-forward-enable-{id}").into(),
                            ))
                            .small()
                            .checked(enabled)
                            .on_click(cx.listener(
                                move |this, checked: &bool, _, cx| {
                                    this.toggle_forward_rule(id, *checked, cx);
                                },
                            )),
                        ),
                )
                .child(
                    Button::new(ElementId::Name(format!("session-forward-del-{id}").into()))
                        .ghost()
                        .danger()
                        .xsmall()
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.remove_forward_rule(id, cx);
                        })),
                );
            list = list.child(row);
        }
        list.into_any_element()
    }

    /// 渲染新增转发规则表单。
    fn render_forward_draft_form(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let draft = &form.forward_draft;
        let kind = draft.kind;
        let kind_selector = self.render_forward_kind_selector(kind, cx);

        let mut card = v_flex()
            .p_3()
            .gap_2p5()
            .rounded(p.radius.md)
            .border_1()
            .border_color(p.border)
            .bg(p.surface.subtle)
            .child(kind_selector)
            .child(crate::widgets::form_field(
                rust_i18n::t!("forward.name").to_string(),
                draft.name.clone(),
                p,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("forward.bind_host").to_string(),
                        draft.bind_host.clone(),
                        p,
                    )))
                    .child(div().w(rems(8.5)).child(number_form_field(
                        rust_i18n::t!("forward.bind_port").to_string(),
                        false,
                        draft.bind_port.clone(),
                        p,
                    ))),
            );

        if kind != SessionForwardKind::Dynamic {
            card = card.child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("forward.target_host").to_string(),
                        draft.target_host.clone(),
                        p,
                    )))
                    .child(div().w(rems(8.5)).child(number_form_field(
                        rust_i18n::t!("forward.target_port").to_string(),
                        false,
                        draft.target_port.clone(),
                        p,
                    ))),
            );
        }

        if let Some(error) = draft.error.clone() {
            card = card.child(div().text_xs().text_color(p.danger).child(error));
        }

        card.child(
            h_flex().justify_end().child(
                Button::new("session-forward-add")
                    .primary()
                    .small()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                    .label(rust_i18n::t!("forward.add").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.add_forward_rule(window, cx);
                    })),
            ),
        )
        .into_any_element()
    }

    /// 渲染转发类型（本地/远程/动态）切换按钮组。
    fn render_forward_kind_selector(
        &self,
        kind: SessionForwardKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        ButtonGroup::new("session-forward-kind")
            .small()
            .child(
                Button::new("session-forward-mode-local")
                    .small()
                    .label(rust_i18n::t!("forward.local").to_string())
                    .selected(kind == SessionForwardKind::Local),
            )
            .child(
                Button::new("session-forward-mode-remote")
                    .small()
                    .label(rust_i18n::t!("forward.remote").to_string())
                    .selected(kind == SessionForwardKind::Remote),
            )
            .child(
                Button::new("session-forward-mode-dynamic")
                    .small()
                    .label(rust_i18n::t!("forward.dynamic").to_string())
                    .selected(kind == SessionForwardKind::Dynamic),
            )
            .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                let next = match clicks.first() {
                    Some(0) => Some(SessionForwardKind::Local),
                    Some(1) => Some(SessionForwardKind::Remote),
                    Some(2) => Some(SessionForwardKind::Dynamic),
                    _ => None,
                };
                if let Some(next) = next {
                    this.edit(cx, |form| form.forward_draft.kind = next);
                }
            }))
            .into_any_element()
    }

    /// 校验草稿并追加一条端口转发规则。
    pub(crate) fn add_forward_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.state_mut() else {
            return;
        };
        match build_rule_from_draft(&form.forward_draft, cx) {
            Ok(rule) => {
                form.forward_draft.error = None;
                form.forward_draft
                    .name
                    .update(cx, |state, cx| state.set_value("", window, cx));
                form.forwards.push(rule);
            }
            Err(error) => {
                form.forward_draft.error = Some(error);
            }
        }
        cx.notify();
    }

    /// 删除指定转发规则。
    pub(crate) fn remove_forward_rule(&mut self, id: ForwardId, cx: &mut Context<Self>) {
        self.edit(cx, |form| {
            form.forwards.retain(|rule| rule.id != id);
        });
    }

    /// 切换指定转发规则随连接自动启动的状态。
    pub(crate) fn toggle_forward_rule(
        &mut self,
        id: ForwardId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |form| {
            if let Some(rule) = form.forwards.iter_mut().find(|rule| rule.id == id) {
                rule.enabled = enabled;
            }
        });
    }
}

/// 从转发草稿读取字段并构造 `ForwardRule`。
fn build_rule_from_draft(
    draft: &crate::session_form::types::SessionForwardDraft,
    cx: &App,
) -> Result<ForwardRule, String> {
    let name = draft.name.read(cx).value().trim().to_string();
    let bind_host = draft.bind_host.read(cx).value().trim().to_string();
    let target_host = draft.target_host.read(cx).value().trim().to_string();
    let bind_port: u16 = draft
        .bind_port
        .read(cx)
        .value()
        .trim()
        .parse()
        .ok()
        .filter(|&port| port > 0)
        .ok_or_else(|| rust_i18n::t!("forward.error_port").to_string())?;
    let target_port: u16 = draft
        .target_port
        .read(cx)
        .value()
        .trim()
        .parse()
        .ok()
        .filter(|&port| port > 0)
        .ok_or_else(|| rust_i18n::t!("forward.error_port").to_string())?;
    if bind_host.is_empty() {
        return Err(rust_i18n::t!("forward.bind_host").to_string());
    }
    if draft.kind != SessionForwardKind::Dynamic && target_host.is_empty() {
        return Err(rust_i18n::t!("forward.error_target").to_string());
    }
    let bind = BindAddress {
        host: bind_host.clone(),
        port: bind_port,
    };
    let kind = match draft.kind {
        SessionForwardKind::Local => ForwardKind::Local {
            bind,
            target: TargetAddress {
                host: target_host,
                port: target_port,
            },
        },
        SessionForwardKind::Remote => ForwardKind::Remote {
            bind,
            target: TargetAddress {
                host: target_host,
                port: target_port,
            },
        },
        SessionForwardKind::Dynamic => ForwardKind::Dynamic {
            bind,
            socks: SocksOptions::default(),
        },
    };
    let rule_name = if name.is_empty() {
        format!("{bind_host}:{bind_port}")
    } else {
        name
    };
    Ok(ForwardRule::new(rule_name, kind))
}

/// 生成单条转发规则的简短摘要。
fn summarize_forward_rule(rule: &ForwardRule) -> String {
    match &rule.kind {
        ForwardKind::Local { bind, target } => {
            format!(
                "L {}:{} -> {}:{}",
                bind.host, bind.port, target.host, target.port
            )
        }
        ForwardKind::Remote { bind, target } => {
            format!(
                "R {}:{} -> {}:{}",
                bind.host, bind.port, target.host, target.port
            )
        }
        ForwardKind::Dynamic { bind, .. } => format!("D {}:{}", bind.host, bind.port),
        _ => String::new(),
    }
}
