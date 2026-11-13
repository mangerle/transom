//! SSH 账户模板选择、内联账户编辑与认证凭据字段。

use super::*;
use crate::identity::IdentityEditState;
use gpui_kit::component::Disableable;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染“账户”行（下拉选择 + 添加账户 + 编辑）及内联账户编辑器。
pub(super) fn render_account_section(
    mut fields: Div,
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> Div {
    let selected_id = form.identity_choice;
    let selected_option = selected_id.and_then(|id| {
        form.identities
            .iter()
            .find(|option| option.id == id)
            .cloned()
    });
    let dropdown_btn = render_account_dropdown(form, selected_id, selected_option.as_ref(), cx);

    let add_btn = Button::new("identity-inline-add")
        .outline()
        .label(rust_i18n::t!("form.account_add").to_string())
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.open_identity_editor(None, window, cx);
        }));

    let edit_btn = Button::new("identity-inline-edit")
        .outline()
        .label(rust_i18n::t!("common.edit").to_string())
        .disabled(selected_id.is_none())
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.open_identity_editor(selected_id, window, cx);
        }));

    fields = fields.child(
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.account").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(dropdown_btn))
                    .child(add_btn)
                    .child(edit_btn),
            ),
    );

    if let Some(edit) = form.identity_edit.as_ref() {
        fields = fields.child(render_inline_identity_editor(edit, cx, p));
    } else if let Some(option) = selected_option {
        fields = fields.child(
            div()
                .px_3()
                .py_2()
                .rounded(p.radius.md)
                .bg(p.muted)
                .text_xs()
                .text_color(p.muted_foreground)
                .child(
                    rust_i18n::t!(
                        "form.account_using_hint",
                        name = &option.name,
                        username = &option.username
                    )
                    .to_string(),
                ),
        );
    }

    fields
}

/// 渲染账户（凭据模板）下拉选择器按钮。
fn render_account_dropdown(
    form: &SessionForm,
    selected_id: Option<transom_core::IdentityId>,
    selected_option: Option<&super::super::types::IdentityOption>,
    cx: &mut Context<SessionFormView>,
) -> AnyElement {
    let current_label = match selected_option {
        Some(option) => format!("{} ({})", option.name, option.username),
        None => rust_i18n::t!("form.account_manual").to_string(),
    };
    let entity = cx.entity();
    let identities = form.identities.clone();

    Button::new("identity-dropdown-trigger")
        .outline()
        .w_full()
        .dropdown_caret(true)
        .label(current_label)
        .dropdown_menu(move |mut menu, _window, _cx| {
            let manual_entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(rust_i18n::t!("form.account_manual").to_string())
                    .checked(selected_id.is_none())
                    .on_click(move |_, _window, cx| {
                        manual_entity.update(cx, |this, cx| {
                            this.edit(cx, |form| form.identity_choice = None);
                        });
                    }),
            );
            for option in &identities {
                let id = option.id;
                let item_entity = entity.clone();
                let label = format!("{} ({})", option.name, option.username);
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(selected_id == Some(id))
                        .on_click(move |_, _window, cx| {
                            item_entity.update(cx, |this, cx| {
                                this.edit(cx, |form| form.identity_choice = Some(id));
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

/// 渲染内联账户编辑器中的认证方式切换栏。
fn render_inline_identity_auth_group(
    auth: AuthChoice,
    cx: &mut Context<SessionFormView>,
) -> AnyElement {
    ButtonGroup::new("inline-identity-auth")
        .small()
        .child(
            Button::new("inline-identity-auth-password")
                .small()
                .label(rust_i18n::t!("form.auth_password").to_string())
                .selected(auth == AuthChoice::Password),
        )
        .child(
            Button::new("inline-identity-auth-key")
                .small()
                .label(rust_i18n::t!("form.auth_key").to_string())
                .selected(auth == AuthChoice::PrivateKey),
        )
        .child(
            Button::new("inline-identity-auth-agent")
                .small()
                .label(rust_i18n::t!("form.auth_agent").to_string())
                .selected(auth == AuthChoice::Agent),
        )
        .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
            let next = match clicks.first() {
                Some(0) => Some(AuthChoice::Password),
                Some(1) => Some(AuthChoice::PrivateKey),
                Some(2) => Some(AuthChoice::Agent),
                _ => None,
            };
            if let Some(next) = next {
                this.edit(cx, |form| {
                    if let Some(edit) = form.identity_edit.as_mut() {
                        edit.auth = next;
                    }
                });
            }
        }))
        .into_any_element()
}

/// 渲染内联账户（凭据模板）新建/编辑卡片。
fn render_inline_identity_editor(
    edit: &IdentityEditState,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let title = if edit.base.is_some() {
        rust_i18n::t!("form.account_edit_title").to_string()
    } else {
        rust_i18n::t!("form.account_new_title").to_string()
    };
    let auth = edit.auth;
    let auth_group = render_inline_identity_auth_group(auth, cx);

    let mut card = v_flex()
        .p_3()
        .gap_2()
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.border)
        .bg(p.surface.subtle)
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(title))
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().child(crate::widgets::form_field(
                    rust_i18n::t!("identity.name").to_string(),
                    edit.name.clone(),
                    p,
                )))
                .child(div().flex_1().child(crate::widgets::form_field(
                    rust_i18n::t!("identity.username").to_string(),
                    edit.username.clone(),
                    p,
                ))),
        )
        .child(auth_group);

    match auth {
        AuthChoice::Password => {
            card = card.child(crate::widgets::secret_field(
                rust_i18n::t!("form.password_optional").to_string(),
                edit.password.clone(),
                p,
            ));
        }
        AuthChoice::PrivateKey => {
            card = card
                .child(crate::widgets::form_field(
                    rust_i18n::t!("identity.key_path").to_string(),
                    edit.key_path.clone(),
                    p,
                ))
                .child(crate::widgets::secret_field(
                    rust_i18n::t!("form.passphrase_optional").to_string(),
                    edit.passphrase.clone(),
                    p,
                ));
        }
        AuthChoice::Agent => {}
    }

    if let Some(error) = edit.error.clone() {
        card = card.child(div().text_xs().text_color(p.danger).child(error));
    }

    card.child(
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("inline-identity-cancel")
                    .ghost()
                    .small()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.cancel_identity_editor(cx);
                    })),
            )
            .child(
                Button::new("inline-identity-save")
                    .primary()
                    .small()
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.save_identity_editor(cx);
                    })),
            ),
    )
    .into_any_element()
}

/// 渲染 SSH 认证方式对应的密码或私钥输入区（输入非空即保存，留空则连接时询问）。
pub(super) fn render_auth_credential_fields(
    mut fields: Div,
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> Div {
    match form.auth {
        AuthChoice::Password => fields.child(crate::widgets::secret_field(
            rust_i18n::t!("form.password_optional").to_string(),
            form.password.clone(),
            p,
        )),
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
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
                                .label(rust_i18n::t!("form.browse").to_string())
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pick_private_key_path(window, cx);
                                })),
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
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
                                .label(rust_i18n::t!("form.browse").to_string())
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pick_certificate_path(window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.cert_hint").to_string()),
                )
                .child(crate::widgets::secret_field(
                    rust_i18n::t!("form.passphrase_optional").to_string(),
                    form.passphrase.clone(),
                    p,
                ));
            fields
        }
        AuthChoice::Agent => fields.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.agent_hint").to_string()),
        ),
    }
}
