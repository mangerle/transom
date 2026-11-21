//! 会话表单中的 FTP 连接配置渲染。
//!
//! FTP 没有交互式 Shell，字段只有主机、端口、用户名、密码与传输安全档位；
//! 密码复用 SSH 的密码输入框（同一时间只有一种连接类型的字段生效），安全
//! 档位按钮组与 Telnet 编码按钮组共用同一布局基元，保持表单视觉一致。

use super::proxy::render_proxy_section;
use super::serial::serial_option_row;
use super::ssh::{number_form_field, required_form_field};
use crate::palette::Palette;
use crate::session_form::{SessionForm, SessionFormView};
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::FtpSecurity;

/// 渲染 FTP 的“基本信息”区（主机、端口、用户名、密码与传输安全档位）。
pub(crate) fn render_ftp_basic_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let security_buttons = h_flex().gap_1().children(
        [
            (FtpSecurity::Plain, "FTP"),
            (FtpSecurity::ExplicitTls, "FTPS"),
        ]
        .into_iter()
        .map(|(security, label)| {
            Button::new(ElementId::Name(format!("ftp-security-{}", label).into()))
                .small()
                .label(label)
                .selected(form.ftp_security == security)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit(cx, |form| form.ftp_security = security);
                }))
        }),
    );

    v_flex()
        .gap_3()
        .child(
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
        )
        .child(required_form_field(
            rust_i18n::t!("form.username").to_string(),
            form.username.clone(),
            p,
        ))
        .child(required_form_field(
            rust_i18n::t!("form.password").to_string(),
            form.password.clone(),
            p,
        ))
        .child(serial_option_row(
            rust_i18n::t!("form.ftp_security").to_string(),
            security_buttons,
            p,
        ))
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.ftp_hint").to_string()),
        )
        .into_any_element()
}

/// 渲染 FTP 的“网络与代理”区（出站代理与被动模式开关）。
///
/// FTP 没有连接超时/keepalive 表单项：拨号超时由协议实现内建（与 SSH 握手
/// 同口径），控制连接保活交给断线自愈的按需重连，不再暴露给用户。
pub(crate) fn render_ftp_network_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_3()
        .child(render_proxy_section(form, cx, p))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .py_1()
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("form.ftp_passive").to_string()),
                )
                .child(
                    Switch::new("toggle-ftp-passive")
                        .small()
                        .checked(form.ftp_passive)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.ftp_passive = *checked);
                        })),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.ftp_passive_hint").to_string()),
        )
        .into_any_element()
}
