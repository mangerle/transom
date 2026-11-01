//! 复选开关 + 固定标签 + 控件的表单行。
//!
//! 设计原理（Why）：批量编辑、备份等表单反复出现“勾选后才启用这一项”的
//! 行结构，每处手写 Checkbox、标签宽度与控件占位，勾选状态与启用逻辑容易
//! 各写一套。组件只负责排版与勾选回调，不接管业务状态。
//!
//! 代价与局限：控件由调用方传入，行内不做禁用联动；是否禁用由调用方根据
//! 勾选值自行决定并传入。

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::*;

/// 标签列宽（rem，标准档 72px）：与批量编辑原有排版一致。
const CHECKBOX_LABEL_WIDTH: f32 = 4.5;

/// 渲染“勾选 + 标签 + 控件”行。
pub(crate) fn checkbox_field_row(
    id: impl Into<ElementId>,
    checked: bool,
    label: impl Into<SharedString>,
    control: AnyElement,
    on_toggle: impl Fn(&bool, &mut Window, &mut App) + 'static,
    _p: &Palette,
) -> AnyElement {
    h_flex()
        .gap(rems(space::SM))
        .items_center()
        .child(
            Checkbox::new(id)
                .small()
                .checked(checked)
                .on_click(move |checked, window, cx| on_toggle(checked, window, cx)),
        )
        .child(
            div()
                .w(rems(CHECKBOX_LABEL_WIDTH))
                .text_size(rems(typography::BODY))
                .child(label.into()),
        )
        .child(div().flex_1().child(control))
        .into_any_element()
}
