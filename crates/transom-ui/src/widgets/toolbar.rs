//! 工具栏容器与分隔原语。

use crate::palette::Palette;
use crate::theme::tokens::{control, space};
use gpui_kit::base::h_flex;
use gpui_kit::*;

/// 标准工具栏：水平排列、紧凑间距、垂直居中。
///
/// 返回 `Div` 而不是 `AnyElement`，调用方可以继续链式追加 `.when(...)`、
/// `.child(...)` 与弹层/拖拽处理。
pub(crate) fn toolbar() -> Div {
    h_flex().w_full().gap(rems(space::XS)).items_center()
}

/// 工具栏竖向分割线：宽度跟随全局分割线设置，高度对齐紧凑图标，避免各处手写 12/14/16px 散落值。
pub(crate) fn toolbar_separator(p: &Palette) -> Div {
    div()
        .flex_none()
        .w(px(p.divider_width.pixels()))
        .h(rems(control::COMPACT_ICON))
        .bg(p.divider(p.border))
}
