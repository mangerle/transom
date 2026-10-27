//! 凭据模板列表与编辑表单视图渲染。

use super::types::{IdentityEditState, describe_auth};
use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_form::AuthChoice;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

impl AppShell {
    /// 凭据模板设置组渲染。
    pub(crate) fn render_settings_identities(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let identities = self.catalog.identities.clone();
        let edit = self.modals.settings.identity_edit.clone();

        let mut list = v_flex().gap_1();
        if identities.is_empty() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("identity.empty").to_string()),
            );
        }
        for identity in identities {
            let id = identity.id;
            let users = self.catalog.identity_users(id);
            let row = h_flex()
                .id(ElementId::Name(format!("identity-row-{id}").into()))
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(p.border.opacity(0.5))
                .child(Icon::new(IconName::KeyRound).with_size(px(14.0)))
                .child(div().flex_1().text_sm().child(format!(
                    "{} · {}",
                    identity.name,
                    describe_auth(&identity.auth)
                )))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("identity.used_by", count = users).to_string()),
                )
                .child(
                    Button::new(ElementId::Name(format!("identity-edit-{id}").into()))
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("common.edit").to_string())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_identity_editor(Some(id), window, cx);
                        })),
                )
                .child(
                    Button::new(ElementId::Name(format!("identity-del-{id}").into()))
                        .ghost()
                        .danger()
                        .xsmall()
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_identity(id, cx);
                        })),
                );
            list = list.child(row);
        }

        let new_button = Button::new("identity-new")
            .small()
            .label(rust_i18n::t!("identity.new").to_string())
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_identity_editor(None, window, cx);
            }));

        let body = v_flex().gap_2().child(list).child(new_button);
        match edit {
            Some(edit) => v_flex()
                .gap_3()
                .child(body)
                .child(self.render_identity_editor(edit, p, cx))
                .into_any_element(),
            None => body.into_any_element(),
        }
    }

    fn render_identity_editor(
        &mut self,
        edit: IdentityEditState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let auth = edit.auth;
        let chip = |label: String, choice: AuthChoice, key: &'static str| {
            Button::new(ElementId::Name(key.into()))
                .small()
                .label(label)
                .selected(auth == choice)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(edit) = this.modals.settings.identity_edit.as_mut() {
                        edit.auth = choice;
                    }
                    cx.notify();
                }))
        };
        let mut content = v_flex()
            .gap_2()
            .p_3()
            .rounded(p.radius.md)
            .border_1()
            .border_color(p.border)
            .child(Self::form_field(
                rust_i18n::t!("identity.name").to_string(),
                edit.name.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("identity.username").to_string(),
                edit.username.clone(),
                p,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(chip(
                        rust_i18n::t!("identity.auth_password").to_string(),
                        AuthChoice::Password,
                        "identity-auth-password",
                    ))
                    .child(chip(
                        rust_i18n::t!("identity.auth_key").to_string(),
                        AuthChoice::PrivateKey,
                        "identity-auth-key",
                    ))
                    .child(chip(
                        rust_i18n::t!("identity.auth_agent").to_string(),
                        AuthChoice::Agent,
                        "identity-auth-agent",
                    )),
            );

        match auth {
            AuthChoice::Password => {
                content = content
                    .child(Self::secret_field(
                        rust_i18n::t!("identity.password").to_string(),
                        edit.password.clone(),
                        p,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child(rust_i18n::t!("identity.save_password").to_string()),
                            )
                            .child(
                                Switch::new("identity-toggle-save-password")
                                    .small()
                                    .checked(edit.save_password)
                                    .on_click(cx.listener(|this, &checked, _, cx| {
                                        if let Some(edit) =
                                            this.modals.settings.identity_edit.as_mut()
                                        {
                                            edit.save_password = checked;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            AuthChoice::PrivateKey => {
                content = content
                    .child(Self::form_field(
                        rust_i18n::t!("identity.key_path").to_string(),
                        edit.key_path.clone(),
                        p,
                    ))
                    .child(Self::secret_field(
                        rust_i18n::t!("identity.passphrase").to_string(),
                        edit.passphrase.clone(),
                        p,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child(rust_i18n::t!("identity.save_passphrase").to_string()),
                            )
                            .child(
                                Switch::new("identity-toggle-save-passphrase")
                                    .small()
                                    .checked(edit.save_passphrase)
                                    .on_click(cx.listener(|this, &checked, _, cx| {
                                        if let Some(edit) =
                                            this.modals.settings.identity_edit.as_mut()
                                        {
                                            edit.save_passphrase = checked;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            AuthChoice::Agent => {}
        }

        if let Some(error) = edit.error.clone() {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }
        content
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("identity-cancel")
                            .ghost()
                            .label(rust_i18n::t!("common.cancel").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_identity_edit(cx);
                            })),
                    )
                    .child(
                        Button::new("identity-save")
                            .primary()
                            .label(rust_i18n::t!("common.save").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_identity(cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
