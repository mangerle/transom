//! 列表行与表头的统一外壳。
//!
//! 只负责行高、内边距、选中/悬浮表面与分割线；行内内容、点击、拖拽与右键
//! 菜单仍由调用方链式追加，避免把业务交互塞进表现组件。

use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::space;
use gpui_kit::base::h_flex;
use gpui_kit::*;

/// 构造一条可选中的列表行。
pub(crate) fn list_row(id: impl Into<ElementId>, selected: bool, p: &Palette) -> Stateful<Div> {
    let row = h_flex()
        .id(id.into())
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(rems(space::SM))
        .items_center()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width);
    if selected {
        row.bg(p.list_active)
            .text_color(p.sidebar_accent_foreground)
    } else {
        row.hover(|style| style.bg(p.list_hover))
    }
}

/// 表头行：弱色小字与底部分割线，供设置表、审计表等复用。
pub(crate) fn table_header(
    children: impl IntoIterator<Item = AnyElement>,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(rems(space::SM))
        .items_center()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .text_size(rems(crate::theme::tokens::typography::CAPTION))
        .text_color(p.muted_foreground)
        .children(children)
        .into_any_element()
}
