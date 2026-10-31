//! SFTP 弹窗视图。

use super::common::*;

impl AppShell {
    pub(crate) fn render_sftp_confirm_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.confirm.as_ref() else {
            return div().into_any_element();
        };
        let title = modal.title.clone();
        let description = modal.description.clone();

        let content = div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(description);
        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-sftp-delete",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                this.sftp_modals.confirm = None;
                this.focus_active_sftp_panel(window, cx);
                cx.notify();
            }),
        );
        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-sftp-delete",
            rust_i18n::t!("common.confirm_delete").to_string(),
            None,
            crate::widgets::DialogConfirmKind::Danger,
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                this.sftp_confirm_delete(cx);
                this.focus_active_sftp_panel(window, cx);
            }),
        );
        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "sftp-confirm-modal",
            title,
            None::<String>,
            content,
            Some(footer),
        )
    }
}
