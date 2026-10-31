//! SFTP 弹窗视图。

use super::common::*;

impl AppShell {
    pub(crate) fn render_sftp_filter_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.filter.as_ref() else {
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
                    this.sftp_modals.filter = None;
                    this.focus_active_sftp_panel(window, cx);
                    cx.notify();
                }),
            ))
            .child(
                Button::new("save-sftp-filter")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("sftp.modals.filter_save").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, window, cx| {
                            this.save_sftp_filter_modal(window, cx);
                        },
                    )),
            );

        crate::widgets::dialog_body(
            "sftp-filter-modal",
            rust_i18n::t!("sftp.filter_rules").to_string(),
            Some(rust_i18n::t!("sftp.modals.filter_scope_hint").to_string()),
            content,
            Some(footer),
        )
    }
}
