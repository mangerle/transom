//! 会话表单中的 Telnet 连接配置渲染。
//!
//! Telnet 无内建认证，字段只有主机、端口、字符编码与连接选项；编码按钮组
//! 与串口的选项行共用同一布局基元，保持表单视觉一致。

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
use transom_core::TextEncoding;

/// 渲染 Telnet 的“基本信息”区（主机、端口、字符编码与说明）。
pub(crate) fn render_telnet_basic_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let encoding_buttons = h_flex().gap_1().children(
        [
            (TextEncoding::Utf8, "UTF-8"),
            (TextEncoding::Gbk, "GBK"),
            (TextEncoding::Big5, "Big5"),
        ]
        .into_iter()
        .map(|(encoding, label)| {
            Button::new(ElementId::Name(format!("telnet-encoding-{}", label).into()))
                .small()
                .label(label)
                .selected(form.telnet_encoding == encoding)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit(cx, |form| form.telnet_encoding = encoding);
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
        .child(serial_option_row(
            rust_i18n::t!("form.telnet_encoding").to_string(),
            encoding_buttons,
            p,
        ))
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.telnet_hint").to_string()),
        )
        .into_any_element()
}

/// 渲染 Telnet 的“网络与代理”区（出站代理、超时保活与自动重连）。
pub(crate) fn render_telnet_network_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_3()
        .child(render_proxy_section(form, cx, p))
        .child(
            h_flex()
                .gap_3()
                .items_end()
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
                    Switch::new("toggle-telnet-auto-reconnect")
                        .small()
                        .checked(form.auto_reconnect)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.auto_reconnect = *checked);
                        })),
                ),
        )
        .into_any_element()
}
