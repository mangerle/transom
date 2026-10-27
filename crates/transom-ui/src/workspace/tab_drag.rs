//! 标签栏拖拽换序：拖拽载荷、跟随预览、左右落点与分组顺序调整。
//!
//! 单独成模块的原因：换序是一套独立的交互（载荷、落点命中、顺序重排），
//! 与标签内容渲染耦合度低，混在 `tab_strip.rs` 会让那个文件超出规模约束。

use crate::app::{AppShell, TabGroup};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::GroupId;

/// 标签拖拽载荷：拖动对象是整个分组，即标签栏上的一个标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabDrag(pub(super) GroupId);

/// 跟随鼠标的标签预览卡片。
pub(super) struct TabDragPreview {
    title: String,
    icon: IconName,
}

impl TabDragPreview {
    fn new(title: String, icon: IconName) -> Self {
        Self { title, icon }
    }
}

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .text_sm()
            .text_color(p.popover_foreground)
            .child(Icon::new(self.icon).xsmall().text_color(p.muted_foreground))
            .child(self.title.clone())
    }
}

/// 给标签挂上拖拽源与左右换序落点。
///
/// 落点是标签的左右两半，不需要用户瞄准细边缘：左半插到该标签之前，右半之后。
/// 覆盖层只在拖拽落下时参与分发，标签中部的点击激活、右键菜单与关闭按钮
/// 仍是原来的命中元素；插入线借 `drag_over` 描边，不需要额外维护悬停状态。
pub(super) fn attach_tab_drag(
    tab: Stateful<Div>,
    group_id: GroupId,
    title: String,
    icon: IconName,
    app: &Entity<AppShell>,
    p: &Palette,
) -> Stateful<Div> {
    tab.relative()
        .on_drag(TabDrag(group_id), move |_, _, _, cx| {
            cx.new(|_| TabDragPreview::new(title.clone(), icon))
        })
        .child(tab_reorder_overlay(app, group_id, p))
}

/// 覆盖整个标签的换序容器：左右两半各承担一个落点。
fn tab_reorder_overlay(app: &Entity<AppShell>, anchor: GroupId, p: &Palette) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_row()
        .child(tab_reorder_half(app, anchor, true, p))
        .child(tab_reorder_half(app, anchor, false, p))
        .into_any_element()
}

/// 半区落点：`left` 决定插到锚点之前还是之后，插入线画在对应一侧。
fn tab_reorder_half(
    app: &Entity<AppShell>,
    anchor: GroupId,
    left: bool,
    p: &Palette,
) -> AnyElement {
    // `drag_over` 的闭包要求 `'static`，颜色先取成局部值再捕获。
    let line_color = p.tab_active_foreground;
    let half = div().flex_1().border_color(line_color);
    let half = if left {
        half.drag_over::<TabDrag>(move |style, _, _, _| style.border_l_2().border_color(line_color))
    } else {
        half.drag_over::<TabDrag>(move |style, _, _, _| style.border_r_2().border_color(line_color))
    };
    let app = app.clone();
    half.on_drop(move |drag: &TabDrag, window, cx| {
        let dragged = drag.0;
        app.update(cx, |this, cx| {
            reorder_groups(&mut this.workspace.groups, dragged, anchor, !left);
            cx.notify();
            window.refresh();
        });
    })
    .into_any_element()
}

/// 把分组 `dragged` 移到 `anchor` 之前或之后，只调整标签顺序。
///
/// `groups` 的数组顺序就是标签栏显示顺序，因此换序无需额外序号字段；活动标签、
/// 窗格布局与焦点都不受影响。锚点下标在摘走自身后会左移一位，必须同步修正，
/// 否则插队位置会偏一格——这也是把纯计算单独抽出来便于单测的原因。
fn reorder_groups(groups: &mut Vec<TabGroup>, dragged: GroupId, anchor: GroupId, after: bool) {
    let Some(anchor_index) = groups.iter().position(|group| group.id == anchor) else {
        return;
    };
    let Some(source_index) = groups.iter().position(|group| group.id == dragged) else {
        return;
    };
    if source_index == anchor_index {
        return;
    }
    let group = groups.remove(source_index);
    let anchor_index = if source_index < anchor_index {
        anchor_index - 1
    } else {
        anchor_index
    };
    let insert_at = if after {
        (anchor_index + 1).min(groups.len())
    } else {
        anchor_index
    };
    groups.insert(insert_at, group);
}

#[cfg(test)]
mod tests {
    use super::reorder_groups;
    use crate::app::TabGroup;
    use transom_core::{GroupId, SessionId, TabId};

    fn three_groups() -> (Vec<TabGroup>, Vec<GroupId>) {
        let groups: Vec<TabGroup> = (0..3)
            .map(|_| TabGroup::new(TabId::new(), SessionId::new(), 1))
            .collect();
        let ids = groups.iter().map(|group| group.id).collect();
        (groups, ids)
    }

    fn ids_of(groups: &[TabGroup]) -> Vec<GroupId> {
        groups.iter().map(|group| group.id).collect()
    }

    /// 前后插入的下标必须精确，尤其是摘走自身后锚点左移的那一位。
    #[test]
    fn reorder_places_group_relative_to_anchor() {
        let (mut groups, ids) = three_groups();

        reorder_groups(&mut groups, ids[2], ids[0], false);
        assert_eq!(ids_of(&groups), vec![ids[2], ids[0], ids[1]]);

        reorder_groups(&mut groups, ids[2], ids[1], true);
        assert_eq!(ids_of(&groups), vec![ids[0], ids[1], ids[2]]);

        // 拖到最后一个标签之后即落到末尾。
        reorder_groups(&mut groups, ids[0], ids[2], true);
        assert_eq!(ids_of(&groups), vec![ids[1], ids[2], ids[0]]);
    }

    /// 拖到自身、或锚点已不存在时不得改动顺序。
    #[test]
    fn reorder_ignores_self_and_unknown_anchor() {
        let (mut groups, ids) = three_groups();

        reorder_groups(&mut groups, ids[1], ids[1], false);
        assert_eq!(ids_of(&groups), ids);

        reorder_groups(&mut groups, GroupId::new(), ids[0], true);
        assert_eq!(ids_of(&groups), ids);
    }
}
