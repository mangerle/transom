//! 会话表单中的 RDP 远程桌面连接配置渲染。
//!
//! 设计原理（Why）：仅展示已真正接入协议栈并生效的配置项（主机、端口、用户名、域、
//! 密码、色深、音频重定向与双向剪贴板），彻底杜绝未生效的假开关或填后丢弃的空壳表单。

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

/// 统一的开关配置行（消除表单中的重复布局样板）。
fn rdp_switch_row(
    id: &'static str,
    label: String,
    checked: bool,
    on_toggle: impl Fn(&mut SessionForm, bool) + 'static,
    cx: &mut Context<SessionFormView>,
) -> AnyElement {
    h_flex()
        .justify_between()
        .items_center()
        .py_1()
        .child(div().text_sm().child(label))
        .child(
            Switch::new(id)
                .small()
                .checked(checked)
                .on_click(cx.listener(move |this, val: &bool, _, cx| {
                    let next = *val;
                    this.edit(cx, |form| on_toggle(form, next));
                })),
        )
        .into_any_element()
}

/// 渲染色深选择按钮组。
fn render_color_depth_picker(
    selected: RdpColorDepth,
    cx: &mut Context<SessionFormView>,
) -> AnyElement {
    h_flex()
        .gap_1()
        .children(
            [
                (RdpColorDepth::Depth16, "16-bit"),
                (RdpColorDepth::Depth24, "24-bit"),
                (RdpColorDepth::Depth32, "32-bit"),
            ]
            .into_iter()
            .map(|(depth, label)| {
                Button::new(ElementId::Name(format!("rdp-depth-{label}").into()))
                    .small()
                    .label(label)
                    .selected(selected == depth)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.edit(cx, |form| form.rdp_color_depth = depth);
                    }))
            }),
        )
        .into_any_element()
}

/// 渲染 RDP 的“基本信息”区（主机、端口、用户名、域、密码、色深与生效特性开关）。
pub(crate) fn render_rdp_basic_fields(
    form: &SessionForm,
    cx: &mut Context<SessionFormView>,
    p: &Palette,
) -> AnyElement {
    let color_depth_buttons = render_color_depth_picker(form.rdp_color_depth, cx);

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
        .child(rdp_switch_row(
            "toggle-rdp-audio",
            rust_i18n::t!("form.rdp_audio").to_string(),
            form.rdp_audio,
            |form, v| form.rdp_audio = v,
            cx,
        ))
        .child(rdp_switch_row(
            "toggle-rdp-clipboard",
            rust_i18n::t!("form.rdp_clipboard").to_string(),
            form.rdp_clipboard,
            |form, v| form.rdp_clipboard = v,
            cx,
        ))
        .into_any_element()
}
