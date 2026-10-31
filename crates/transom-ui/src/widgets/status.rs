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
        .child(div().size(rems(space::SM)).rounded_full().bg(color))
        .into_any_element()
}
