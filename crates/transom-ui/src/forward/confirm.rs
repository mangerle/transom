//! 端口转发公网/冲突确认弹窗视图。

use crate::app::{AppShell, AppState};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;

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
