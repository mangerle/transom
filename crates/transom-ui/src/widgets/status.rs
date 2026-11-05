//! 状态点与状态徽标。

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

/// 纯文本状态徽标：胶囊底、语义色文字。
pub(crate) fn status_badge(label: impl Into<SharedString>, color: Hsla, p: &Palette) -> AnyElement {
    div()
        .px(rems(space::SM))
        .py(rems(space::XXS))
        .rounded(p.radius.sm)
        .bg(color.opacity(0.12))
        .text_size(rems(typography::CAPTION))
        .text_color(color)
        .child(label.into())
        .into_any_element()
}

/// 无 tooltip 的状态圆点：列表行、标签与徽标内的纯装饰点。
///
/// 尺寸走 `space::SM` 令牌；此前各页面分别写 6/7/8px，密度切换与并排对齐
/// 都会漂移。
pub(crate) fn status_dot_bare(color: Hsla) -> Div {
    div()
        .size(rems(space::SM))
        .rounded_full()
        .flex_none()
        .bg(color)
}

/// 状态圆点：用于会话行尾的拨测状态。
pub(crate) fn status_dot(
    id: impl Into<ElementId>,
    color: Hsla,
    tooltip: impl Into<SharedString>,
) -> AnyElement {
    let tooltip = tooltip.into();
    div()
        .id(id.into())
        .flex_none()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(status_dot_bare(color))
        .into_any_element()
}

/// 通用 4px 进度条：拉取、构建、导入导出共用。
///
/// 轨道使用 8% 弱色透明度，填充使用指定的主色/语义色，胶囊全圆。
pub(crate) fn progress_bar(fraction: f32, color: Hsla, p: &Palette) -> AnyElement {
    let progress_val = fraction.clamp(0.0, 1.0);
    div()
        .w_full()
        .h(rems(space::XS))
        .rounded_full()
        .bg(p.foreground.opacity(0.08))
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(progress_val)),
        )
        .into_any_element()
}

