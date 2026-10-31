//! 工具栏容器。

use crate::theme::tokens::space;
use gpui_kit::base::h_flex;
use gpui_kit::*;

/// 标准工具栏：水平排列、紧凑间距、垂直居中。
///
/// 返回 `Div` 而不是 `AnyElement`，调用方可以继续链式追加 `.when(...)`、
/// `.child(...)` 与弹层/拖拽处理。
pub(crate) fn toolbar() -> Div {
    h_flex().w_full().gap(rems(space::XS)).items_center()
}
