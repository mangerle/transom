//! 纵向标签行的拖拽换序：上下半区落点与跟随预览。
//!
//! 与横向标签栏共用同一份拖拽载荷（`TabDrag`）与顺序调整函数
//! （`reorder_groups`），只把落点从左右两半换成上下两半：上半区插到锚点
//! 之前，下半区插到锚点之后。数组顺序仍是唯一事实来源，换序不触碰活动
//! 标签、分屏布局与焦点。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::workspace::tab_drag::{TabDrag, TabDragPreview, reorder_groups};
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::*;
use transom_core::GroupId;

/// 给纵向标签行挂上拖拽源与上下换序落点。
pub(super) fn attach_vertical_tab_drag(
    row: Stateful<Div>,
    group_id: GroupId,
    title: String,
    icon: IconName,
    app: &Entity<AppShell>,
    p: &Palette,
) -> Stateful<Div> {
    row.relative()
        .on_drag(TabDrag(group_id), move |_, _, _, cx| {
            cx.new(|_| TabDragPreview::new(title.clone(), icon))
        })
        .child(vertical_reorder_overlay(app, group_id, p))
}

/// 覆盖整行的换序容器：上下两半各承担一个落点。
fn vertical_reorder_overlay(app: &Entity<AppShell>, anchor: GroupId, p: &Palette) -> AnyElement {
    v_flex()
        .absolute()
        .inset_0()
        .child(vertical_reorder_half(app, anchor, true, p))
        .child(vertical_reorder_half(app, anchor, false, p))
        .into_any_element()
}

/// 半区落点：`top` 决定插到锚点之前还是之后，插入线画在对应一侧。
fn vertical_reorder_half(
    app: &Entity<AppShell>,
    anchor: GroupId,
    top: bool,
    p: &Palette,
) -> AnyElement {
    // `drag_over` 的闭包要求 `'static`，颜色先取成局部值再捕获。
    let line_color = p.tab_active_foreground;
    let half = div().flex_1().border_color(line_color);
    let half = if top {
        half.drag_over::<TabDrag>(move |style, _, _, _| style.border_t_2().border_color(line_color))
    } else {
        half.drag_over::<TabDrag>(move |style, _, _, _| style.border_b_2().border_color(line_color))
    };
    let app = app.clone();
    half.on_drop(move |drag: &TabDrag, window, cx| {
        let dragged = drag.0;
        app.update(cx, |this, cx| {
            reorder_groups(&mut this.workspace.groups, dragged, anchor, !top);
            cx.notify();
            window.refresh();
        });
    })
    .into_any_element()
}
