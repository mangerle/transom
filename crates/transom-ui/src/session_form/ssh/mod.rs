//! 会话表单中 SSH / Mosh 基本信息与网络代理参数渲染模块。

use super::SessionFormView;
use super::proxy::render_proxy_section;
use super::types::{AuthChoice, ConnectionChoice, SessionForm};
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, NumberInput};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
mod auth;
mod jump;

use self::auth::{render_account_section, render_auth_credential_fields};
use self::jump::render_jump_section;

/// 带红色必填星号标记的文本输入字段。
pub(super) fn required_form_field(
    label: String,
    input: Entity<InputState>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .gap_0p5()
                .items_center()
                .child(div().text_xs().text_color(p.muted_foreground).child(label))
                .child(div().text_xs().text_color(p.danger).child("*")),
        )
        .child(Input::new(&input).w_full())
        .into_any_element()
}

/// 使用 `NumberInput`（带加减步进按钮）的数字输入字段。
pub(super) fn number_form_field(
    label: String,
    required: bool,
    input: Entity<InputState>,
    p: &Palette,
) -> AnyElement {
    let header = if required {
        h_flex()
            .gap_0p5()
            .items_center()
            .child(div().text_xs().text_color(p.muted_foreground).child(label))
            .child(div().text_xs().text_color(p.danger).child("*"))
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(label)
            .into_any_element()
    };
    v_flex()
        .gap_1()
        .child(header)
        .child(NumberInput::new(&input).w_full())
        .into_any_element()
}

/// 渲染 SSH / Mosh 的“基本信息”区（主机、端口、账户、用户名与认证方式）。
pub(crate) fn render_ssh_basic_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let auth_selector = ButtonGroup::new("auth-methods")
        .small()
        .child(
            Button::new("auth-password")
                .small()
                .label(rust_i18n::t!("form.auth_password").to_string())
                .selected(form.auth == AuthChoice::Password),
        )
        .child(
            Button::new("auth-key")
                .small()
                .label(rust_i18n::t!("form.auth_key").to_string())
                .selected(form.auth == AuthChoice::PrivateKey),
        )
        .child(
            Button::new("auth-agent")
                .small()
                .label(rust_i18n::t!("form.auth_agent").to_string())
                .selected(form.auth == AuthChoice::Agent),
        )
        .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
            let choice = match clicks.first() {
                Some(0) => Some(AuthChoice::Password),
                Some(1) => Some(AuthChoice::PrivateKey),
                Some(2) => Some(AuthChoice::Agent),
                _ => None,
            };
            if let Some(choice) = choice {
                this.edit(cx, |form| form.auth = choice);
            }
        }));

    let mut fields = v_flex().gap_3().child(
        h_flex()
            .gap_3()
            .items_end()
            .child(div().flex_1().child(required_form_field(
                rust_i18n::t!("form.host").to_string(),
                form.host.clone(),
                p,
            )))
            .child(div().w(rems(8.5)).child(number_form_field(
                rust_i18n::t!("form.port").to_string(),
                true,
                form.port.clone(),
                p,
            ))),
    );

    fields = render_account_section(fields, form, cx, p);

    if form.identity_choice.is_none() {
        fields = fields
            .child(required_form_field(
                rust_i18n::t!("form.username").to_string(),
                form.username.clone(),
                p,
            ))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.auth").to_string()),
                    )
                    .child(auth_selector),
            );
        fields = render_auth_credential_fields(fields, form, cx, p);
    }

    fields.into_any_element()
}

/// 渲染 SSH / Mosh 的“网络与代理”区（跳板机、出站代理、超时保活、重连与高级网络项）。
pub(crate) fn render_ssh_network_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let mut fields = v_flex().gap_3();

    if form.connection == ConnectionChoice::Ssh {
        fields = fields.child(render_jump_section(form, cx, p));
    }
    fields = fields.child(render_proxy_section(form, cx, p));

    fields = fields
        .child(
            h_flex()
                .gap_3()
                .child(div().flex_1().child(number_form_field(
                    rust_i18n::t!("form.connect_timeout").to_string(),
                    false,
                    form.connect_timeout.clone(),
                    p,
                )))
                .child(div().flex_1().child(number_form_field(
                    rust_i18n::t!("form.keepalive").to_string(),
                    false,
                    form.keepalive.clone(),
                    p,
                ))),
        )
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
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.auto_reconnect = *checked);
                        })),
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
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.agent_forward = *checked);
                        })),
                ),
        );

    if form.connection == ConnectionChoice::Ssh {
        fields = fields.child(crate::widgets::form_field(
            rust_i18n::t!("form.wol_mac").to_string(),
            form.wol_mac.clone(),
            p,
        ));
    } else if form.connection == ConnectionChoice::Mosh {
        fields = fields
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("form.mosh.server_path").to_string(),
                        form.mosh_server_path.clone(),
                        p,
                    )))
                    .child(div().w(rems(10.0)).child(crate::widgets::form_field(
                        rust_i18n::t!("form.mosh.server_port").to_string(),
                        form.mosh_server_port.clone(),
                        p,
                    ))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.mosh.hint").to_string()),
            );
    }

    fields.into_any_element()
}
