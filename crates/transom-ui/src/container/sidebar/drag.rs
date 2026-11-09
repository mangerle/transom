//! 引擎侧栏拖拽：落点判定与排序遮罩。
//!
//! 设计原理（Why）：拖放动作本身要写会话目录，属于外壳领域；侧栏实体只把
//! 载荷与落点包装成事件上交，渲染函数负责命中区域与视觉反馈。

use super::*;

/// 引擎行/分组行的排序落点遮罩：会话行二分，分组行保留中段作为“放入该分组”。
pub(crate) fn engine_reorder_overlay(
    view: &Entity<ContainerSidebarView>,
    anchor: ContainerSidebarDrag,
    keep_middle: bool,
    p: &Palette,
) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(engine_reorder_half(view, anchor, DropSide::Before, true, p));
    let overlay = if keep_middle {
        overlay.child(div().flex_1())
    } else {
        overlay
    };
    overlay
        .child(engine_reorder_half(view, anchor, DropSide::After, false, p))
        .into_any_element()
}

pub(crate) fn engine_reorder_half(
    view: &Entity<ContainerSidebarView>,
    anchor: ContainerSidebarDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = ContainerSidebarTarget::Relative { anchor, side };
    let line_color = p.sidebar_accent_foreground;
    let zone = div().flex_1().border_color(line_color);
    let zone = if top {
        zone.drag_over::<ContainerSidebarDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color)
        })
    } else {
        zone.drag_over::<ContainerSidebarDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color)
        })
    };
    let view = view.clone();
    zone.on_drop(move |drag: &ContainerSidebarDrag, window, cx| {
        let drag = *drag;
        view.update(cx, |sidebar, cx| {
            sidebar.request_drop(drag, target, cx);
        });
        window.refresh();
    })
    .into_any_element()
}
