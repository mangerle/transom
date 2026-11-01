//! 开关卡片：标题 + 说明 + 右侧开关的常用设置行。
//!
//! 设计原理（Why）：标签/窗口等设置页反复出现“整块卡片内放说明与开关”
//! 的结构，每处手写内边距、圆角、双语说明与回调；组件把排版与点击语义
//! 固定下来，页面只提供文案与状态。
//!
//! 代价与局限：开关回调仍由调用方写入设置；组件不感知设置存储。

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

/// 开关切换回调：由页面用 `cx.listener` 构造，保留实体上下文。
pub(crate) type SwitchToggle = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// 开关卡片参数：标识、文案、当前值与切换回调。
pub(crate) struct SwitchCardProps {
    pub(crate) id: &'static str,
    pub(crate) title: SharedString,
    pub(crate) description: SharedString,
    pub(crate) checked: bool,
    pub(crate) on_toggle: SwitchToggle,
}

/// 渲染开关卡片。
pub(crate) fn switch_card(props: SwitchCardProps, p: &Palette) -> AnyElement {
    let SwitchCardProps {
        id,
        title,
        description,
        checked,
        on_toggle,
    } = props;
    h_flex()
        .w_full()
        .min_w_0()
        .justify_between()
        .items_center()
        .gap(rems(space::SM))
        .py(rems(space::XS))
        .px(rems(space::SM))
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.border)
        .bg(p.popover)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_size(rems(typography::BODY)).child(title))
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.muted_foreground)
                        .child(description),
                ),
        )
        .child(
            Switch::new(id)
                .small()
                .checked(checked)
                .on_click(move |checked, window, cx| on_toggle(checked, window, cx)),
        )
        .into_any_element()
}
