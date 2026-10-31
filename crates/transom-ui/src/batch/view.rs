//! 批量编辑弹窗视图。

use super::common::*;

impl AppShell {
    pub(crate) fn render_batch_edit_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.modals.batch_edit.as_ref() else {
            return div().into_any_element();
        };
        let selected_count = form.selected_count;
        let change_port = form.change_port;
        let change_username = form.change_username;
        let change_auth = form.change_auth;
        let auth = form.auth;
        let save_passphrase = form.save_passphrase;
        let error = form.error.clone();

        let port_row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Checkbox::new("batch-change-port")
                    .small()
                    .checked(change_port)
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, checked: &bool, _, cx| {
                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                form.change_port = *checked;
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .w(px(72.0))
                    .text_sm()
                    .child(rust_i18n::t!("form.port").to_string()),
            )
            .child(div().flex_1().child(Input::new(&form.port)));

        let username_row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Checkbox::new("batch-change-username")
                    .small()
                    .checked(change_username)
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, checked: &bool, _, cx| {
                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                form.change_username = *checked;
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .w(px(72.0))
                    .text_sm()
                    .child(rust_i18n::t!("form.username").to_string()),
            )
            .child(div().flex_1().child(Input::new(&form.username)));

        let auth_buttons = h_flex()
            .gap_2()
            .child(
                Button::new("batch-auth-password")
                    .small()
                    .label(rust_i18n::t!("form.auth_password").to_string())
                    .selected(auth == AuthChoice::Password)
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                form.auth = AuthChoice::Password;
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(
                Button::new("batch-auth-key")
                    .small()
                    .label(rust_i18n::t!("form.auth_key").to_string())
                    .selected(auth == AuthChoice::PrivateKey)
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                form.auth = AuthChoice::PrivateKey;
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(
                Button::new("batch-auth-agent")
                    .small()
                    .label(rust_i18n::t!("form.auth_agent").to_string())
                    .selected(auth == AuthChoice::Agent)
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                form.auth = AuthChoice::Agent;
                            }
                            cx.notify();
                        },
                    )),
            );

        let mut auth_section = v_flex().gap_2().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Checkbox::new("batch-change-auth")
                        .small()
                        .checked(change_auth)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, checked: &bool, _, cx| {
                                if let Some(form) = this.modals.batch_edit.as_mut() {
                                    form.change_auth = *checked;
                                }
                                cx.notify();
                            },
                        )),
                )
                .child(
                    div()
                        .w(px(72.0))
                        .text_sm()
                        .child(rust_i18n::t!("form.auth").to_string()),
                )
                .child(auth_buttons),
        );
        if change_auth {
            match auth {
                AuthChoice::Password => {
                    auth_section = auth_section.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("batch.password_hint").to_string()),
                    );
                }
                AuthChoice::PrivateKey => {
                    auth_section = auth_section.child(Input::new(&form.key_path)).child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child(rust_i18n::t!("form.save_passphrase").to_string()),
                            )
                            .child(
                                Switch::new("batch-save-passphrase")
                                    .small()
                                    .checked(save_passphrase)
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        |this, checked: &bool, _, cx| {
                                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                                form.save_passphrase = *checked;
                                            }
                                            cx.notify();
                                        },
                                    )),
                            ),
                    );
                }
                AuthChoice::Agent => {}
            }
        }

        let mut content = v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("batch.hint", count = selected_count).to_string()),
            )
            .child(port_row)
            .child(username_row)
            .child(auth_section);
        if let Some(error) = error {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }

        let footer = h_flex()
            .gap_2()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-batch-edit",
                rust_i18n::t!("common.cancel").to_string(),
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.modals.batch_edit = None;
                    cx.notify();
                }),
            ))
            .child(
                Button::new("apply-batch-edit")
                    .primary()
                    .label(rust_i18n::t!("batch.apply").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.save_batch_edit(cx),
                    )),
            );

        crate::widgets::dialog_body(
            "batch-edit-modal",
            rust_i18n::t!("batch.title").to_string(),
            Some(rust_i18n::t!("batch.apply_hint").to_string()),
            content,
            Some(footer),
        )
    }
}
