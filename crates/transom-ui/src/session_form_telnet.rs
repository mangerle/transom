//! 会话表单中的 Telnet 连接配置渲染。
//!
//! Telnet 无内建认证，字段只有主机、端口、字符编码与连接选项；编码按钮组
//! 与串口的选项行共用同一布局基元，保持表单视觉一致。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_form::SessionForm;
use crate::session_form_serial::serial_option_row;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::TextEncoding;

pub(crate) fn render_telnet_fields(
    form: &SessionForm,
    cx: &mut Context<AppShell>,
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this.form.as_mut() {
                        form.telnet_encoding = encoding;
                    }
                    cx.notify();
                }))
        }),
    );

    v_flex()
        .gap_3()
        .child(
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
        )
        .child(serial_option_row(
            rust_i18n::t!("form.telnet_encoding").to_string(),
            encoding_buttons,
            p,
        ))
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
                        .on_click(cx.listener(|this, &checked, _, cx| {
                            if let Some(form) = this.form.as_mut() {
                                form.auto_reconnect = checked;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .items_end()
                .child(div().flex_1().child(AppShell::form_field(
                    rust_i18n::t!("form.connect_timeout").to_string(),
                    form.connect_timeout.clone(),
                    p,
                )))
                .child(div().flex_1().child(AppShell::form_field(
                    rust_i18n::t!("form.keepalive").to_string(),
                    form.keepalive.clone(),
                    p,
                ))),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.telnet_hint").to_string()),
        )
        .into_any_element()
}
