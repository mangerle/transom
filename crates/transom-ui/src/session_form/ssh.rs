//! 会话表单中 SSH 连接参数、跳板机与认证凭据渲染模块。

use super::proxy::render_proxy_section;
use super::types::{AuthChoice, SessionForm};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

mod auth;
mod jump;

use self::auth::render_auth_credential_fields;
use self::jump::render_jump_section;

pub(crate) fn render_ssh_fields(
    form: &SessionForm,
    app: &Entity<AppShell>,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let auth_password = if form.auth == AuthChoice::Password {
        Button::new("auth-password")
            .primary()
            .small()
            .label(rust_i18n::t!("form.auth_password").to_string())
    } else {
        Button::new("auth-password")
            .small()
            .label(rust_i18n::t!("form.auth_password").to_string())
    }
    .on_click(crate::dialog_host::on_event(
        app,
        |this, _: &ClickEvent, _, cx| {
            this.update_session_form(cx, |form| form.auth = AuthChoice::Password);
        },
    ));

    let auth_key = if form.auth == AuthChoice::PrivateKey {
        Button::new("auth-key")
            .primary()
            .small()
            .label(rust_i18n::t!("form.auth_key").to_string())
    } else {
        Button::new("auth-key")
            .small()
            .label(rust_i18n::t!("form.auth_key").to_string())
    }
    .on_click(crate::dialog_host::on_event(
        app,
        |this, _: &ClickEvent, _, cx| {
            this.update_session_form(cx, |form| form.auth = AuthChoice::PrivateKey);
        },
    ));

    let auth_agent = if form.auth == AuthChoice::Agent {
        Button::new("auth-agent")
            .primary()
            .small()
            .label(rust_i18n::t!("form.auth_agent").to_string())
    } else {
        Button::new("auth-agent")
            .small()
            .label(rust_i18n::t!("form.auth_agent").to_string())
    }
    .on_click(crate::dialog_host::on_event(
        app,
        |this, _: &ClickEvent, _, cx| {
            this.update_session_form(cx, |form| form.auth = AuthChoice::Agent);
        },
    ));

    let mut fields = v_flex().gap_3();

    fields = fields.child(
        h_flex()
            .gap_3()
            .items_end()
            .child(div().flex_1().child(crate::widgets::form_field(
                rust_i18n::t!("form.host").to_string(),
                form.host.clone(),
                p,
            )))
            .child(div().w(px(110.0)).child(crate::widgets::form_field(
                rust_i18n::t!("form.port").to_string(),
                form.port.clone(),
                p,
            ))),
    );

    fields = fields.child(render_jump_section(form, app, cx, p));
    fields = fields.child(render_proxy_section(form, app, cx, p));

    if !form.identities.is_empty() {
        fields = fields.child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.identity").to_string()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("identity-none")
                                .small()
                                .label(rust_i18n::t!("form.identity_none").to_string())
                                .selected(form.identity_choice.is_none())
                                .on_click(crate::dialog_host::on_event(
                                    app,
                                    |this, _: &ClickEvent, _, cx| {
                                        this.update_session_form(cx, |form| {
                                            form.identity_choice = None;
                                        });
                                    },
                                )),
                        )
                        .children(form.identities.iter().enumerate().map(|(index, option)| {
                            let identity_id = option.id;
                            let selected = form.identity_choice == Some(identity_id);
                            Button::new(ElementId::Name(format!("identity-{index}").into()))
                                .small()
                                .label(option.name.clone())
                                .selected(selected)
                                .on_click(crate::dialog_host::on_event(
                                    app,
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.update_session_form(cx, |form| {
                                            form.identity_choice = Some(identity_id);
                                        });
                                    },
                                ))
                        })),
                ),
        );
    }

    if form.identity_choice.is_none() {
        fields = fields
            .child(crate::widgets::form_field(
                rust_i18n::t!("form.username").to_string(),
                form.username.clone(),
                p,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.auth").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(auth_password)
                            .child(auth_key)
                            .child(auth_agent),
                    ),
            );
    }

    fields = fields
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .py_1()
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("form.auto_reconnect").to_string()),
                )
                .child(
                    Switch::new("toggle-auto-reconnect")
                        .small()
                        .checked(form.auto_reconnect)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, checked: &bool, _, cx| {
                                this.update_session_form(cx, |form| {
                                    form.auto_reconnect = *checked;
                                });
                            },
                        )),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .py_1()
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("form.agent_forward").to_string()),
                )
                .child(
                    Switch::new("toggle-agent-forward")
                        .small()
                        .checked(form.agent_forward)
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, checked: &bool, _, cx| {
                                this.update_session_form(cx, |form| {
                                    form.agent_forward = *checked;
                                });
                            },
                        )),
                ),
        );

    fields = fields
        .child(crate::widgets::form_field(
            rust_i18n::t!("form.connect_timeout").to_string(),
            form.connect_timeout.clone(),
            p,
        ))
        .child(crate::widgets::form_field(
            rust_i18n::t!("form.keepalive").to_string(),
            form.keepalive.clone(),
            p,
        ))
        .child(crate::widgets::form_field(
            rust_i18n::t!("form.wol_mac").to_string(),
            form.wol_mac.clone(),
            p,
        ));

    fields = render_auth_credential_fields(fields, form, app, cx, p);

    fields.into_any_element()
}
