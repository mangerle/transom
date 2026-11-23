//! 会话表单中的 RDP 远程桌面连接配置渲染。
//!
//! 设计原理（Why）：RDP 具备独立的图形分辨率、色深与重定向通道（音频、剪贴板、
//! 文件传输）；此处提供直观的配置表单，并与现有的代理设置保持统一交互规范。

use super::proxy::render_proxy_section;
use super::serial::serial_option_row;
use super::ssh::{number_form_field, required_form_field};
use crate::palette::Palette;
use crate::session_form::{SessionForm, SessionFormView};
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::RdpColorDepth;

/// 无必填标记的常规文本输入字段。
fn optional_form_field(label: String, input: Entity<InputState>, p: &Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(p.muted_foreground).child(label))
        .child(Input::new(&input).w_full())
        .into_any_element()
}

/// 渲染 RDP 的“基本信息”区（主机、端口、用户名、域、密码、色深与特性开关）。
pub(crate) fn render_rdp_basic_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let color_depth_buttons = h_flex().gap_1().children(
        [
            (RdpColorDepth::Depth16, "16-bit"),
            (RdpColorDepth::Depth24, "24-bit"),
            (RdpColorDepth::Depth32, "32-bit"),
        ]
        .into_iter()
        .map(|(depth, label)| {
            Button::new(ElementId::Name(format!("rdp-depth-{}", label).into()))
                .small()
                .label(label)
                .selected(form.rdp_color_depth == depth)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit(cx, |form| form.rdp_color_depth = depth);
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
        .child(
            h_flex()
                .gap_3()
                .items_end()
                .child(div().flex_1().child(optional_form_field(
                    rust_i18n::t!("form.username").to_string(),
                    form.username.clone(),
                    p,
                )))
                .child(div().flex_1().child(optional_form_field(
                    rust_i18n::t!("form.rdp_domain").to_string(),
                    form.rdp_domain.clone(),
                    p,
                ))),
        )
        .child(optional_form_field(
            rust_i18n::t!("form.password").to_string(),
            form.password.clone(),
            p,
        ))
        .child(serial_option_row(
            rust_i18n::t!("form.rdp_color_depth").to_string(),
            color_depth_buttons,
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
                        .child(rust_i18n::t!("form.rdp_audio").to_string()),
                )
                .child(
                    Switch::new("toggle-rdp-audio")
                        .small()
                        .checked(form.rdp_audio)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.rdp_audio = *checked);
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
                        .child(rust_i18n::t!("form.rdp_clipboard").to_string()),
                )
                .child(
                    Switch::new("toggle-rdp-clipboard")
                        .small()
                        .checked(form.rdp_clipboard)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.rdp_clipboard = *checked);
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
                        .child(rust_i18n::t!("form.rdp_file_copy").to_string()),
                )
                .child(
                    Switch::new("toggle-rdp-file-copy")
                        .small()
                        .checked(form.rdp_file_copy)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.edit(cx, |form| form.rdp_file_copy = *checked);
                        })),
                ),
        )
        .into_any_element()
}

/// 渲染 RDP 的“网络与代理”区。
pub(crate) fn render_rdp_network_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_3()
        .child(render_proxy_section(form, cx, p))
        .into_any_element()
}
