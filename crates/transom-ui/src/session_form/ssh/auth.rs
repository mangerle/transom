//! SSH 认证方式与凭据字段。

use super::*;

/// 渲染 SSH 认证方式对应的密码或私钥输入区与保存开关。
pub(super) fn render_auth_credential_fields(
    mut fields: Div,
    form: &SessionForm,
    app: &Entity<AppShell>,
    _cx: &App,
    p: &Palette,
) -> Div {
    match form.auth {
        AuthChoice::Password => {
            fields = fields.child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("form.save_password").to_string()),
                    )
                    .child(
                        Switch::new("toggle-save-password")
                            .small()
                            .checked(form.save_password)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, checked: &bool, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.save_password = *checked;
                                    }
                                    cx.notify();
                                },
                            )),
                    ),
            );
            if form.save_password {
                fields.child(crate::widgets::secret_field(
                    rust_i18n::t!("form.password").to_string(),
                    form.password.clone(),
                    p,
                ))
            } else {
                fields.child(credential_deferred_hint(p))
            }
        }
        AuthChoice::PrivateKey => {
            fields = fields
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(div().flex_1().child(crate::widgets::form_field(
                            rust_i18n::t!("form.key_path").to_string(),
                            form.key_path.clone(),
                            p,
                        )))
                        .child(
                            Button::new("browse-key-path")
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label(rust_i18n::t!("form.browse").to_string())
                                .on_click(crate::dialog_host::on_event(
                                    app,
                                    |this, _: &ClickEvent, window, cx| {
                                        this.pick_private_key_path(window, cx);
                                    },
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(div().flex_1().child(crate::widgets::form_field(
                            rust_i18n::t!("form.cert_path").to_string(),
                            form.cert_path.clone(),
                            p,
                        )))
                        .child(
                            Button::new("browse-cert-path")
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label(rust_i18n::t!("form.browse").to_string())
                                .on_click(crate::dialog_host::on_event(
                                    app,
                                    |this, _: &ClickEvent, window, cx| {
                                        this.pick_certificate_path(window, cx);
                                    },
                                )),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.cert_hint").to_string()),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .py_1()
                        .child(
                            div()
                                .text_sm()
                                .child(rust_i18n::t!("form.save_passphrase").to_string()),
                        )
                        .child(
                            Switch::new("toggle-save-passphrase")
                                .small()
                                .checked(form.save_passphrase)
                                .on_click(crate::dialog_host::on_event(
                                    app,
                                    |this, checked: &bool, _, cx| {
                                        if let Some(form) = this.modals.form.as_mut() {
                                            form.save_passphrase = *checked;
                                        }
                                        cx.notify();
                                    },
                                )),
                        ),
                );
            if form.save_passphrase {
                fields.child(crate::widgets::secret_field(
                    rust_i18n::t!("form.passphrase").to_string(),
                    form.passphrase.clone(),
                    p,
                ))
            } else {
                fields.child(credential_deferred_hint(p))
            }
        }
        AuthChoice::Agent => fields,
    }
}

/// "本次不保存凭据"的说明行：告诉用户密码会在连接时询问，而不是漏填。
fn credential_deferred_hint(p: &Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("form.credential_deferred").to_string())
        .into_any_element()
}
