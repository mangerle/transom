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

pub(crate) fn render_ssh_fields(
    form: &SessionForm,
    cx: &mut Context<AppShell>,
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
    .on_click(cx.listener(|this, _, _, cx| {
        if let Some(form) = this.modals.form.as_mut() {
            form.auth = AuthChoice::Password;
        }
        cx.notify();
    }));

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
    .on_click(cx.listener(|this, _, _, cx| {
        if let Some(form) = this.modals.form.as_mut() {
            form.auth = AuthChoice::PrivateKey;
        }
        cx.notify();
    }));

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
    .on_click(cx.listener(|this, _, _, cx| {
        if let Some(form) = this.modals.form.as_mut() {
            form.auth = AuthChoice::Agent;
        }
        cx.notify();
    }));

    let mut fields = v_flex().gap_3();

    fields = fields.child(
        h_flex()
            .gap_3()
            .items_end()
            .child(div().flex_1().child(AppShell::form_field(
                rust_i18n::t!("form.host").to_string(),
                form.host.clone(),
                p,
            )))
            .child(div().w(px(110.0)).child(AppShell::form_field(
                rust_i18n::t!("form.port").to_string(),
                form.port.clone(),
                p,
            ))),
    );

    fields = fields.child(render_jump_section(form, cx, p));
    fields = fields.child(render_proxy_section(form, cx, p));

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
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.identity_choice = None;
                                    }
                                    cx.notify();
                                })),
                        )
                        .children(form.identities.iter().enumerate().map(|(index, option)| {
                            let identity_id = option.id;
                            let selected = form.identity_choice == Some(identity_id);
                            Button::new(ElementId::Name(format!("identity-{index}").into()))
                                .small()
                                .label(option.name.clone())
                                .selected(selected)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.identity_choice = Some(identity_id);
                                    }
                                    cx.notify();
                                }))
                        })),
                ),
        );
    }

    if form.identity_choice.is_none() {
        fields = fields
            .child(AppShell::form_field(
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
                        .on_click(cx.listener(|this, &checked, _, cx| {
                            if let Some(form) = this.modals.form.as_mut() {
                                form.auto_reconnect = checked;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .py_1()
                .child(div().text_sm().child("SSH Agent 代理转发 (ssh -A)"))
                .child(
                    Switch::new("toggle-agent-forward")
                        .small()
                        .checked(form.agent_forward)
                        .on_click(cx.listener(|this, &checked, _, cx| {
                            if let Some(form) = this.modals.form.as_mut() {
                                form.agent_forward = checked;
                            }
                            cx.notify();
                        })),
                ),
        );

    fields = fields
        .child(AppShell::form_field(
            rust_i18n::t!("form.connect_timeout").to_string(),
            form.connect_timeout.clone(),
            p,
        ))
        .child(AppShell::form_field(
            rust_i18n::t!("form.keepalive").to_string(),
            form.keepalive.clone(),
            p,
        ))
        .child(AppShell::form_field(
            rust_i18n::t!("form.wol_mac").to_string(),
            form.wol_mac.clone(),
            p,
        ));

    fields = render_auth_credential_fields(fields, form, cx, p);

    fields.into_any_element()
}

/// 渲染跳板机级联下拉选择与拓扑链展示。
fn render_jump_section(form: &SessionForm, cx: &mut Context<AppShell>, p: &Palette) -> AnyElement {
    let jump_label = form
        .jump_choice
        .and_then(|id| {
            form.jump_options
                .iter()
                .find(|option| option.id == id)
                .map(|option| option.name.clone())
        })
        .unwrap_or_else(|| rust_i18n::t!("form.proxy_jump_none").to_string());
    let target_host = form.host.read(cx).value().trim().to_string();
    let target_port = form.port.read(cx).value().trim().to_string();
    let target_label = if target_port.is_empty() || target_port == "22" {
        target_host
    } else {
        format!("{target_host}:{target_port}")
    };
    let topology = form.proxy_topology(&target_label);
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.proxy_jump").to_string()),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Button::new("proxy-jump-dropdown")
                        .outline()
                        .small()
                        .icon(IconName::Route)
                        .label(jump_label)
                        .dropdown_menu({
                            let options = form.jump_options.clone();
                            let app_entity = cx.entity().clone();
                            move |menu, _window, _cx| {
                                let app = app_entity.clone();
                                let mut menu = menu.item(
                                    PopupMenuItem::new(
                                        rust_i18n::t!("form.proxy_jump_none").to_string(),
                                    )
                                    .icon(IconName::Ban)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, _, cx| {
                                            app.update(cx, |this, cx| {
                                                if let Some(form) = this.modals.form.as_mut() {
                                                    form.jump_choice = None;
                                                }
                                                cx.notify();
                                            });
                                        }
                                    }),
                                );
                                if !options.is_empty() {
                                    menu = menu.item(PopupMenuItem::separator());
                                }
                                for option in &options {
                                    let id = option.id;
                                    let label = format!(
                                        "{} ({}:{})",
                                        option.name, option.host, option.port
                                    );
                                    let app = app.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(label).icon(IconName::Route).on_click(
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(form) = this.modals.form.as_mut() {
                                                        form.jump_choice = Some(id);
                                                    }
                                                    cx.notify();
                                                });
                                            },
                                        ),
                                    );
                                }
                                menu
                            }
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(topology),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.proxy_jump_hint").to_string()),
        )
        .into_any_element()
}

/// 渲染 SSH 认证方式对应的密码或私钥输入区与保存开关。
fn render_auth_credential_fields(
    mut fields: Div,
    form: &SessionForm,
    cx: &mut Context<AppShell>,
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
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                if let Some(form) = this.modals.form.as_mut() {
                                    form.save_password = checked;
                                }
                                cx.notify();
                            })),
                    ),
            );
            if form.save_password {
                fields.child(AppShell::secret_field(
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
                        .child(div().flex_1().child(AppShell::form_field(
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
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pick_private_key_path(window, cx);
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(div().flex_1().child(AppShell::form_field(
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
                                .on_click(cx.listener(|this, _, window, cx| {
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
                                .on_click(cx.listener(|this, &checked, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.save_passphrase = checked;
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            if form.save_passphrase {
                fields.child(AppShell::secret_field(
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
