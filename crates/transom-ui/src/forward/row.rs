//! 单条端口转发规则卡片渲染。

use super::ForwardView;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{Disableable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{
    ForwardId, ForwardKind, ForwardMeter, ForwardRule, ForwardState, ForwardStatus, TabId,
};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

impl ForwardView {
    /// 单条转发规则卡片：头部、拓扑、运行动作与结果提示分层组装。
    pub(super) fn render_forward_rule_row(
        &self,
        rule: &ForwardRule,
        statuses: &HashMap<ForwardId, ForwardStatus>,
        tab_id: TabId,
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
        let (kind_text, kind_color) = forward_kind_visual(&rule.kind, p);
        let active_meter = status
            .as_ref()
            .filter(|status| matches!(status.state, ForwardState::Active { .. }))
            .and_then(|status| status.meter.as_ref());

        let mut row = render_forward_card_shell(editing, p)
            .child(render_forward_header(
                ForwardRowHeader {
                    rule,
                    rule_id,
                    tab_id,
                    running,
                    editing,
                    kind_text,
                    kind_color,
                    status_text,
                    status_color,
                    p,
                },
                cx,
            ))
            .child(render_forward_topology(rule, p));

        if let Some(meter) = active_meter.filter(|_| running) {
            row = row.child(render_forward_live_actions(
                ForwardLiveActions {
                    rule,
                    rule_id,
                    tab_id,
                    meter,
                    p,
                },
                cx,
            ));
        }
        if let Some(probe) = self.probe_results().get(&rule_id) {
            row = row.child(render_forward_probe(probe, p));
        }
        if let Some(error) = status.and_then(|status| status.error) {
            row = row.child(render_forward_status_error(&error, p));
        }
        row.into_any_element()
    }
}

/// 规则卡片头部渲染参数。
struct ForwardRowHeader<'a> {
    rule: &'a ForwardRule,
    rule_id: ForwardId,
    tab_id: TabId,
    running: bool,
    editing: bool,
    kind_text: String,
    kind_color: Hsla,
    status_text: String,
    status_color: Hsla,
    p: &'a Palette,
}

/// 运行中规则的动作行参数。
struct ForwardLiveActions<'a> {
    rule: &'a ForwardRule,
    rule_id: ForwardId,
    tab_id: TabId,
    meter: &'a ForwardMeter,
    p: &'a Palette,
}

fn render_forward_card_shell(editing: bool, p: &Palette) -> Div {
    v_flex()
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
}

fn forward_kind_visual(kind: &ForwardKind, p: &Palette) -> (String, Hsla) {
    match kind {
        ForwardKind::Local { .. } => (rust_i18n::t!("forward.local").to_string(), p.primary),
        ForwardKind::Remote { .. } => (rust_i18n::t!("forward.remote").to_string(), p.warning),
        ForwardKind::Dynamic { .. } => (rust_i18n::t!("forward.dynamic").to_string(), p.success),
        _ => (String::new(), p.muted_foreground),
    }
}

fn render_forward_header(
    header: ForwardRowHeader<'_>,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    let ForwardRowHeader {
        rule,
        rule_id,
        tab_id,
        running,
        editing,
        kind_text,
        kind_color,
        status_text,
        status_color,
        p,
    } = header;
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
                .child(render_edit_button(rule, rule_id, running, editing, cx))
                .child(render_start_stop_button(
                    rule_id, tab_id, running, editing, cx,
                ))
                .child(render_delete_button(rule_id, tab_id, cx)),
        )
        .into_any_element()
}

fn render_edit_button(
    rule: &ForwardRule,
    rule_id: ForwardId,
    running: bool,
    editing: bool,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    Button::new(ElementId::Name(format!("forward-edit-{rule_id}").into()))
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
            move |view, _, window, cx| view.begin_edit(&rule, window, cx)
        }))
        .into_any_element()
}

fn render_start_stop_button(
    rule_id: ForwardId,
    tab_id: TabId,
    running: bool,
    editing: bool,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    if running {
        Button::new(ElementId::Name(format!("forward-stop-{rule_id}").into()))
            .small()
            .label(rust_i18n::t!("forward.stop").to_string())
            .on_click(cx.listener(move |_view, _, _, cx| {
                cx.emit(super::ForwardPanelEvent::StopRule { tab_id, rule_id });
                cx.notify();
            }))
    } else {
        Button::new(ElementId::Name(format!("forward-start-{rule_id}").into()))
            .small()
            .outline()
            .label(rust_i18n::t!("forward.start").to_string())
            .disabled(editing)
            .tooltip(rust_i18n::t!("forward.editing_cannot_start").to_string())
            .on_click(cx.listener(move |_view, _, _, cx| {
                cx.emit(super::ForwardPanelEvent::StartRule { tab_id, rule_id });
                cx.notify();
            }))
    }
    .into_any_element()
}

fn render_delete_button(
    rule_id: ForwardId,
    tab_id: TabId,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    Button::new(ElementId::Name(format!("forward-delete-{rule_id}").into()))
        .ghost()
        .small()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
        .on_click(cx.listener(move |_view, _, _, cx| {
            cx.emit(super::ForwardPanelEvent::DeleteRule { tab_id, rule_id });
            cx.notify();
        }))
        .into_any_element()
}

fn render_forward_topology(rule: &ForwardRule, p: &Palette) -> AnyElement {
    match &rule.kind {
        ForwardKind::Local { bind, target } | ForwardKind::Remote { bind, target } => h_flex()
            .gap_1p5()
            .items_center()
            .text_xs()
            .child(forward_endpoint_chip(&bind.host, bind.port, p))
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::ARROW_RIGHT))
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(forward_endpoint_chip(&target.host, target.port, p))
            .into_any_element(),
        ForwardKind::Dynamic { bind, .. } => h_flex()
            .gap_1p5()
            .items_center()
            .text_xs()
            .child(forward_endpoint_chip(&bind.host, bind.port, p))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child("SOCKS5"),
            )
            .into_any_element(),
        _ => div().into_any_element(),
    }
}

fn forward_endpoint_chip(host: &str, port: u16, p: &Palette) -> AnyElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(p.radius.sm)
        .bg(p.surface.ghost)
        .border_1()
        .border_color(p.stroke.faint)
        .text_color(p.foreground)
        .font_family(p.mono_font_family.clone())
        .child(format!("{host}:{port}"))
        .into_any_element()
}

fn render_forward_live_actions(
    actions: ForwardLiveActions<'_>,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    let ForwardLiveActions {
        rule,
        rule_id,
        tab_id,
        meter,
        p,
    } = actions;
    let (bytes, connections) = meter.snapshot();
    let mut row = h_flex().gap_2().items_center().child(
        div().text_xs().text_color(p.muted_foreground).child(
            rust_i18n::t!(
                "forward.traffic",
                traffic = transom_core::format_traffic(bytes)
            )
            .to_string(),
        ),
    );
    if connections > 0 {
        row = row.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("forward.connections", count = connections).to_string()),
        );
    }
    row = row
        .child(render_probe_button(rule_id, tab_id, cx))
        .child(render_copy_cli_button(rule_id, tab_id, cx));
    if rule.browser_url().is_some() {
        row = row.child(render_browser_button(rule_id, tab_id, cx));
    }
    row.into_any_element()
}

fn render_probe_button(
    rule_id: ForwardId,
    tab_id: TabId,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    Button::new(ElementId::Name(format!("forward-probe-{rule_id}").into()))
        .ghost()
        .small()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::FETCH)).small())
        .tooltip(rust_i18n::t!("forward.test").to_string())
        .on_click(cx.listener(move |_view, _, _, cx| {
            cx.emit(super::ForwardPanelEvent::ProbeRule { tab_id, rule_id });
            cx.notify();
        }))
        .into_any_element()
}

fn render_copy_cli_button(
    rule_id: ForwardId,
    tab_id: TabId,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    Button::new(ElementId::Name(format!("forward-cli-{rule_id}").into()))
        .ghost()
        .small()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::COPY)).small())
        .tooltip(rust_i18n::t!("forward.copy_cli").to_string())
        .on_click(cx.listener(move |_view, _, _, cx| {
            cx.emit(super::ForwardPanelEvent::CopyCli { tab_id, rule_id });
            cx.notify();
        }))
        .into_any_element()
}

fn render_browser_button(
    rule_id: ForwardId,
    tab_id: TabId,
    cx: &mut Context<ForwardView>,
) -> AnyElement {
    Button::new(ElementId::Name(format!("forward-browser-{rule_id}").into()))
        .ghost()
        .small()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::EXTERNAL_LINK)).small())
        .tooltip(rust_i18n::t!("forward.open_browser").to_string())
        .on_click(cx.listener(move |_view, _, _, cx| {
            cx.emit(super::ForwardPanelEvent::OpenBrowser { tab_id, rule_id });
            cx.notify();
        }))
        .into_any_element()
}

fn render_forward_probe(probe: &Option<String>, p: &Palette) -> AnyElement {
    match probe {
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
    }
    .into_any_element()
}

fn render_forward_status_error(error: &transom_core::AppError, p: &Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(p.danger)
        .child(localized_error(error))
        .into_any_element()
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
