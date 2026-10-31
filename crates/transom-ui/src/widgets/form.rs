//! 表单与设置行的统一骨骼。
//!
//! 只定义“标题 + 说明 + 控件”的布局和分割线；控件本身仍由调用方传入，
//! 因此开关、下拉、输入框、按钮都共用同一套行距与对齐。

use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// Zed 风格设置行：左侧标题与说明、右侧控件。
///
/// 宽度约束是这里的重点：左列必须 `flex_1 + min_w_0`，控件必须 `flex_none`。
/// 少任一条，弹性布局就会按内容固有宽度排版，一句长中文说明能把整行撑到
/// 比面板内容区还宽，右侧控件被推到面板外截断。
pub(crate) fn setting_row(
    title_key: &'static str,
    description_key: Option<&'static str>,
    control: AnyElement,
    p: &Palette,
) -> AnyElement {
    setting_text_row(
        rust_i18n::t!(title_key).to_string(),
        description_key.map(|key| rust_i18n::t!(key).to_string()),
        control,
        p,
    )
}

/// 与 [`setting_row`] 同构，但标题与说明由调用方传入已本地化文本。
pub(crate) fn setting_text_row(
    title: String,
    description: Option<String>,
    control: AnyElement,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .w_full()
        .py_2()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .justify_between()
        .items_center()
        .gap_4()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(title))
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(description),
                    )
                }),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// 设置行外框：统一行距并在行底画一条分割线。
pub(crate) fn setting_frame(content: AnyElement, p: &Palette) -> AnyElement {
    v_flex()
        .w_full()
        .py_2()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(content)
        .into_any_element()
}

/// 带分割线的输入行：输入框与其说明同属一行。
pub(crate) fn setting_field_row(
    field: AnyElement,
    note: Option<String>,
    p: &Palette,
) -> AnyElement {
    let inner = v_flex()
        .w_full()
        .gap_1()
        .child(field)
        .when_some(note, |this, note| {
            this.child(div().text_xs().text_color(p.muted_foreground).child(note))
        })
        .into_any_element();
    setting_frame(inner, p)
}

/// 表单字段：标题在上、输入框在下，供会话表单等纵排表单复用。
pub(crate) fn form_field(label: String, input: Entity<InputState>, p: &Palette) -> AnyElement {
    field_with_reveal(label, input, false, p)
}

/// 机密字段（登录密码、私钥口令）：在普通字段之上启用输入框自带的明文切换按钮。
pub(crate) fn secret_field(label: String, input: Entity<InputState>, p: &Palette) -> AnyElement {
    field_with_reveal(label, input, true, p)
}

/// 表单字段的共用实现；`reveal` 决定是否显示输入框自带的掩码切换按钮。
pub(crate) fn field_with_reveal(
    label: String,
    input: Entity<InputState>,
    reveal: bool,
    p: &Palette,
) -> AnyElement {
    let field = Input::new(&input).w_full();
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(p.muted_foreground).child(label))
        .child(if reveal { field.mask_toggle() } else { field })
        .into_any_element()
}
