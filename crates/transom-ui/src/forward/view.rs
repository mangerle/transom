//! 端口转发面板与公网监听确认弹窗视图渲染。

use super::ForwardView;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{Disableable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_core::{ForwardKind, ForwardState};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl ForwardView {
    pub(crate) fn render_forward_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 规则与状态由外壳注入：渲染期不反查外壳。
        let tab_id = self.tab_id();
        let forwards = self.rules().to_vec();
        let statuses = self.statuses().clone();

        let header = self.render_forward_header(cx, p);
        let rules_list = self.render_forward_rules_list(tab_id, &forwards, &statuses, cx, p);
        let add_form = self.render_forward_add_form(tab_id, cx, p);

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.background)
            .text_color(p.foreground)
            .child(header)
            .child(rules_list)
            .child(add_form)
            .into_any_element()
    }

    /// 端口转发面板顶栏标题与关闭按钮。
    ///
    /// 高度取标签栏 token 而不是靠内边距撑开：该面板与主区标签条、监控抽屉
    /// 并排，三条横线必须在同一水平线上。
    fn render_forward_header(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let close = crate::widgets::IconButton::new(
            "close-forward",
            AppIcon::Ui(UiIcon::CLOSE),
            rust_i18n::t!("forward.close_panel").to_string(),
        )
        .size(crate::widgets::IconButtonSize::Compact)
        .render()
        .on_click(cx.listener(|_this, _, _, cx| {
            // 句柄由窗格持有，回收动作回交外壳；实体只发请求。
            cx.emit(super::ForwardPanelEvent::ClosePanel);
            cx.notify();
        }));
        crate::widgets::PanelHeader::new(
            format!("forward-panel-header-{}", self.tab_id()),
            rust_i18n::t!("forward.title").to_string(),
            p,
        )
        .action(close)
        .render(p)
    }

    /// 端口转发规则列表渲染。
    fn render_forward_rules_list(
        &self,
        tab_id: transom_core::TabId,
        rules: &[transom_core::ForwardRule],
        statuses: &std::collections::HashMap<transom_core::ForwardId, transom_core::ForwardStatus>,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut rule_rows: Vec<AnyElement> = Vec::new();
        if rules.is_empty() {
            rule_rows.push(
                div()
                    .py_8()
                    .px_3()
                    .child(crate::widgets::empty_state(
                        AppIcon::Domain(DomainIcon::FORWARD),
                        rust_i18n::t!("forward.empty").to_string(),
                        rust_i18n::t!("forward.empty_description").to_string(),
                        p,
                    ))
                    .into_any_element(),
            );
        }
        for rule in rules {
            rule_rows.push(self.render_forward_rule_row(rule, statuses, tab_id, cx, p));
        }

        v_flex()
            .id(ElementId::Name("forward-rules".into()))
            .w_full()
            .flex_1()
            .overflow_y_scroll()
            .p_2()
            .gap_2()
            .children(rule_rows)
            .into_any_element()
    }

    /// 单条转发规则卡片：头部（状态/启停/编辑/删除）、拓扑行、动作行与错误。
    ///
    /// 卡片化布局吸收自 winssh-pro 的转发对话框：三行结构在 360px 侧面板内
    /// 竖排压缩。编辑中的卡片琥珀色描边对上"改的是哪条"；运行中禁编辑、
    /// 编辑中禁启动，避免编辑态与运行态互相覆盖。
    fn render_forward_rule_row(
        &self,
        rule: &transom_core::ForwardRule,
        statuses: &std::collections::HashMap<transom_core::ForwardId, transom_core::ForwardStatus>,
        tab_id: transom_core::TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let rule_id = rule.id;
        let status = statuses.get(&rule_id).cloned();
        let running = status.as_ref().is_some_and(|status| {
            matches!(
                status.state,
                ForwardState::Active { .. } | ForwardState::Starting
            )
        });
        let editing = self.editing_id() == Some(rule_id);
        let (status_text, status_color) = forward_rule_status(status.as_ref(), p);
        let (kind_text, kind_color) = match &rule.kind {
            ForwardKind::Local { .. } => (rust_i18n::t!("forward.local").to_string(), p.primary),
            ForwardKind::Remote { .. } => (rust_i18n::t!("forward.remote").to_string(), p.warning),
            ForwardKind::Dynamic { .. } => {
                (rust_i18n::t!("forward.dynamic").to_string(), p.success)
            }
            _ => (String::new(), p.muted_foreground),
        };
        let active_meter = status
            .as_ref()
            .filter(|status| matches!(status.state, ForwardState::Active { .. }))
            .and_then(|status| status.meter.as_ref());

        let topology = match &rule.kind {
            ForwardKind::Local { bind, target } | ForwardKind::Remote { bind, target } => h_flex()
                .gap_1p5()
                .items_center()
                .text_xs()
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.surface.ghost)
                        .border_1()
                        .border_color(p.stroke.faint)
                        .text_color(p.foreground)
                        .font_family(p.mono_font_family.clone())
                        .child(format!("{}:{}", bind.host, bind.port)),
                )
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::ARROW_RIGHT))
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.surface.ghost)
                        .border_1()
                        .border_color(p.stroke.faint)
                        .text_color(p.foreground)
                        .font_family(p.mono_font_family.clone())
                        .child(format!("{}:{}", target.host, target.port)),
                )
                .into_any_element(),
            ForwardKind::Dynamic { bind, .. } => h_flex()
                .gap_1p5()
                .items_center()
                .text_xs()
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.surface.ghost)
                        .border_1()
                        .border_color(p.stroke.faint)
                        .text_color(p.foreground)
                        .font_family(p.mono_font_family.clone())
                        .child(format!("{}:{}", bind.host, bind.port)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child("SOCKS5"),
                )
                .into_any_element(),
            _ => div().into_any_element(),
        };

        let mut row = v_flex()
            .w_full()
            .gap_1p5()
            .p_2p5()
            .rounded(p.radius.md)
            .bg(if editing {
                p.tone.warning_subtle
            } else {
                p.surface.faint
            })
            .border_1()
            .border_color(if editing { p.warning } else { p.stroke.subtle })
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(crate::widgets::status_badge(kind_text, kind_color, p))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(rule.name.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(crate::widgets::status_badge(status_text, status_color, p))
                            .child(
                                Button::new(ElementId::Name(
                                    format!("forward-edit-{rule_id}").into(),
                                ))
                                .ghost()
                                .small()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::EDIT)).small())
                                .disabled(running || editing)
                                .tooltip(if running {
                                    rust_i18n::t!("forward.stop_before_edit").to_string()
                                } else {
                                    rust_i18n::t!("forward.edit").to_string()
                                })
                                .on_click(cx.listener({
                                    let rule = rule.clone();
                                    move |this, _, window, cx| {
                                        this.begin_edit(&rule, window, cx);
                                    }
                                })),
                            )
                            .child(if running {
                                Button::new(ElementId::Name(
                                    format!("forward-stop-{rule_id}").into(),
                                ))
                                .small()
                                .label(rust_i18n::t!("forward.stop").to_string())
                                .on_click(cx.listener(
                                    move |_this, _, _, cx| {
                                        cx.emit(super::ForwardPanelEvent::StopRule {
                                            tab_id,
                                            rule_id,
                                        });
                                        cx.notify();
                                    },
                                ))
                            } else {
                                Button::new(ElementId::Name(
                                    format!("forward-start-{rule_id}").into(),
                                ))
                                .small()
                                .outline()
                                .label(rust_i18n::t!("forward.start").to_string())
                                .disabled(editing)
                                .tooltip(rust_i18n::t!("forward.editing_cannot_start").to_string())
                                .on_click(cx.listener(
                                    move |_this, _, _, cx| {
                                        cx.emit(super::ForwardPanelEvent::StartRule {
                                            tab_id,
                                            rule_id,
                                        });
                                        cx.notify();
                                    },
                                ))
                            })
                            .child(
                                Button::new(ElementId::Name(
                                    format!("forward-delete-{rule_id}").into(),
                                ))
                                .ghost()
                                .small()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                                .on_click(cx.listener(
                                    move |_this, _, _, cx| {
                                        cx.emit(super::ForwardPanelEvent::DeleteRule {
                                            tab_id,
                                            rule_id,
                                        });
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
            .child(topology);
        // 动作行：仅运行中有意义——流量/连接数实时读原子值，其余按类型提供。
        if let (Some(meter), true) = (active_meter, running) {
            let (bytes, connections) = meter.snapshot();
            let mut actions = h_flex().gap_2().items_center().child(
                div().text_xs().text_color(p.muted_foreground).child(
                    rust_i18n::t!(
                        "forward.traffic",
                        traffic = transom_core::format_traffic(bytes)
                    )
                    .to_string(),
                ),
            );
            if connections > 0 {
                actions =
                    actions.child(div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!("forward.connections", count = connections).to_string(),
                    ));
            }
            actions = actions
                .child(
                    Button::new(ElementId::Name(format!("forward-probe-{rule_id}").into()))
                        .ghost()
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::FETCH)).small())
                        .tooltip(rust_i18n::t!("forward.test").to_string())
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(super::ForwardPanelEvent::ProbeRule { tab_id, rule_id });
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(ElementId::Name(format!("forward-cli-{rule_id}").into()))
                        .ghost()
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::COPY)).small())
                        .tooltip(rust_i18n::t!("forward.copy_cli").to_string())
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(super::ForwardPanelEvent::CopyCli { tab_id, rule_id });
                            cx.notify();
                        })),
                );
            if rule.browser_url().is_some() {
                actions = actions.child(
                    Button::new(ElementId::Name(format!("forward-browser-{rule_id}").into()))
                        .ghost()
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::EXTERNAL_LINK)).small())
                        .tooltip(rust_i18n::t!("forward.open_browser").to_string())
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(super::ForwardPanelEvent::OpenBrowser { tab_id, rule_id });
                            cx.notify();
                        })),
                );
            }
            row = row.child(actions);
        }
        if let Some(probe) = self.probe_results().get(&rule_id) {
            row = row.child(match probe {
                Some(text) => div()
                    .text_xs()
                    .px_1()
                    .rounded(p.radius.sm)
                    .bg(p.danger.opacity(0.15))
                    .text_color(p.danger)
                    .child(text.clone()),
                None => div()
                    .text_xs()
                    .px_1()
                    .rounded(p.radius.sm)
                    .bg(p.success.opacity(0.15))
                    .text_color(p.success)
                    .child(rust_i18n::t!("forward.test_ok").to_string()),
            });
        }
        if let Some(error) = status.and_then(|status| status.error) {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(p.danger)
                    .child(localized_error(&error)),
            );
        }
        row.into_any_element()
    }
}

/// 转发状态文案与颜色；无状态视为已停止。
fn forward_rule_status(
    status: Option<&transom_core::ForwardStatus>,
    p: &Palette,
) -> (String, Hsla) {
    match status {
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
            ForwardState::NeedsConfirmation => (
                rust_i18n::t!("forward.needs_confirmation").to_string(),
                p.warning,
            ),
            _ => (String::new(), p.muted_foreground),
        },
        None => (
            rust_i18n::t!("forward.stopped").to_string(),
            p.muted_foreground,
        ),
    }
}
