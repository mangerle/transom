//! SFTP 弹窗视图。

use super::common::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

pub(crate) fn render_sftp_filter_body(
    shell: &AppShell,
    app: &Entity<AppShell>,
    _cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(modal) = shell.state.sftp_modals.filter.as_ref() else {
        return div().into_any_element();
    };
    let textarea = modal.textarea.clone();
    let error = modal.error.clone();

    let content = v_flex()
        .gap_3()
        .child(Textarea::new(&textarea).h(px(220.0)).w_full())
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.filter_rules_hint").to_string()),
        )
        .when_some(error, |this, err| {
            this.child(div().text_xs().text_color(p.danger).child(err))
        });

    let footer = h_flex()
        .gap_2()
        .child(crate::widgets::dialog_cancel_button(
            "cancel-sftp-filter",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                this.state.sftp_modals.filter = None;
                crate::sftp::focus_active_sftp_panel(this, window, cx);
                cx.notify();
            }),
        ))
        .child(crate::widgets::dialog_confirm_button(
            "save-sftp-filter",
            rust_i18n::t!("sftp.modals.filter_save").to_string(),
            Some(AppIcon::Ui(UiIcon::CHECK)),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                crate::sftp::save_sftp_filter_modal(this, window, cx);
            }),
        ));

    crate::widgets::dialog_body(
        "sftp-filter-modal",
        rust_i18n::t!("sftp.filter_rules").to_string(),
        Some(rust_i18n::t!("sftp.modals.filter_scope_hint").to_string()),
        content,
        Some(footer),
    )
}
