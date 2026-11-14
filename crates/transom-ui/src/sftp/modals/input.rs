//! SFTP 弹窗视图。

use super::common::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

pub(crate) fn render_sftp_input_body(
    shell: &AppShell,
    app: &Entity<AppShell>,
    _cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(modal) = shell.state.sftp_modals.input.as_ref() else {
        return div().into_any_element();
    };
        let title = modal.title.clone();
        let prompt = modal.prompt.clone();
        let input = modal.input.clone();
        let error = modal.error.clone();

        let content = v_flex()
            .gap_3()
            .child(Input::new(&input).w_full())
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-sftp-input",
                rust_i18n::t!("common.cancel").to_string(),
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                    this.state.sftp_modals.input = None;
                    crate::sftp::focus_active_sftp_panel(this, window, cx);
                    cx.notify();
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "confirm-sftp-input",
                rust_i18n::t!("common.confirm").to_string(),
                Some(AppIcon::Ui(UiIcon::CHECK)),
                crate::widgets::DialogConfirmKind::Primary,
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                    crate::sftp::commit_sftp_input_modal(this, window, cx);
                }),
            ));

        crate::widgets::dialog_body(
            "sftp-input-modal",
            title,
            Some(prompt),
            content,
            Some(footer),
        )
}
