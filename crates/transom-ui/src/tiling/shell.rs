//! AppShell 并排状态增删查改与条目装配实现。

use super::layout::auto_tiling_grid;
use super::tabs::TiledTabs;
use super::types::{MAX_TILING_SLOTS, StripEntry, TilingId, TilingLayoutChoice};
use crate::app::{AppShell, AppState};
use crate::pane_tree::PaneDirection;
use crate::sftp::PendingEditorClose;
use gpui_kit::*;
use transom_core::{GroupId, SplitAxis, TabId};

/// 快速并排：把右键的标签和它在标签栏里左边那一条并到一起。
pub(crate) fn tile_tab(
    app_state: &mut AppState,
    target: GroupId,
    axis: SplitAxis,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(target_pane) = app_state
        .workspace
        .group(target)
        .map(|group| group.active_pane)
    else {
        return;
    };
    if app_state.tiling_of_group(target).is_some() {
        crate::tiling::activate_group(app_state, target, window, cx);
        return;
    }
    let Some(previous) = app_state.previous_strip_entry(target) else {
        return;
    };
    match previous {
        StripEntry::Tiled { id, .. } => {
            if let Some(group) =
                crate::tiling::add_groups_to_tiling(app_state, id, vec![target], window)
            {
                crate::tiling::activate_group(app_state, group, window, cx);
            } else {
                app_state.set_error(rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS));
                cx.notify();
            }
        }
        StripEntry::Single(partner) if partner != target => {
            let (rows, cols) = match axis {
                SplitAxis::Horizontal => (1, 2),
                SplitAxis::Vertical => (2, 1),
            };
            let Some(tiled) = TiledTabs::new(vec![partner, target], rows, cols, false) else {
                return;
            };
            app_state.workspace.tiled_sets.push(tiled);
            app_state.prune_pane_split_states();
            app_state.ui.window_requests.push_activate_tab(target_pane);
        }
        StripEntry::Single(_) => {
            app_state.ui.window_requests.push_activate_tab(target_pane);
        }
    }
}

/// 关闭指定并排组里的全部标签（合并条目关闭菜单的第二项）。
pub(crate) fn close_tiling_group(
    app_state: &mut AppState,
    id: TilingId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(groups) = app_state
        .tiling_by_id(id)
        .map(|tiled| tiled.occupied_groups())
    else {
        return;
    };
    crate::tiling::close_tiling_groups(app_state, groups, window, cx);
}

/// 关闭指定的一组并排标签：先统一过脏编辑闸门，再逐个关闭标签组。
pub(crate) fn close_tiling_groups(
    app_state: &mut AppState,
    groups: Vec<GroupId>,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let panes: Vec<TabId> = groups
        .iter()
        .filter_map(|group_id| {
            app_state
                .workspace
                .groups
                .iter()
                .find(|group| group.id == *group_id)
        })
        .flat_map(|group| group.pane_ids())
        .collect();
    if crate::editor::block_close_for_dirty_editors(
        app_state,
        &panes,
        PendingEditorClose::Tiling(groups.clone()),
        cx,
    ) {
        return;
    }
    for group_id in groups {
        if app_state
            .workspace
            .groups
            .iter()
            .any(|group| group.id == group_id)
        {
            app_state.ui.window_requests.push_close_group(group_id);
        }
    }
}

/// 退出指定并排组，只改变显示方式：组内标签都保持打开与连接。
pub(crate) fn untile_tiling(app_state: &mut AppState, id: TilingId, cx: &mut Context<AppShell>) {
    let before = app_state.workspace.tiled_sets.len();
    app_state
        .workspace
        .tiled_sets
        .retain(|tiled| tiled.id != id);
    if app_state.workspace.tiled_sets.len() != before {
        app_state.prune_pane_split_states();
        app_state.sync_broadcast_targets();
        cx.notify();
    }
}

/// 激活某个标签组（通常来自并排格标题栏或点击格内容）。
pub(crate) fn activate_group(
    app_state: &mut AppState,
    group: GroupId,
    _window: &mut Window,
    _cx: &mut Context<AppShell>,
) {
    let Some(index) = app_state.workspace.group_index(group) else {
        return;
    };
    let pane = app_state.workspace.groups[index].active_pane;
    app_state.ui.window_requests.push_activate_tab(pane);
}

/// 窗格方向键在组内已无邻居时，沿网格方向跨到相邻槽位。
pub(crate) fn focus_neighbor_tile(
    app_state: &mut AppState,
    direction: PaneDirection,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(tiled) = app_state.active_tiling().cloned() else {
        return;
    };
    let Some(active) = app_state.active_group_id() else {
        return;
    };
    let Some(slot) = tiled.slot_of(active) else {
        return;
    };
    let Some(next) = tiled.adjacent_slot(slot, direction) else {
        return;
    };
    let Some(group) = tiled.group_at(next) else {
        return;
    };
    crate::tiling::activate_group(app_state, group, window, cx);
}

/// Ctrl/Cmd+点击标签：整组加入或移出多选集合。
pub(crate) fn toggle_tab_selection(
    app_state: &mut AppState,
    groups: &[GroupId],
    cx: &mut Context<AppShell>,
) {
    if groups.is_empty() {
        return;
    }
    let all_selected = groups
        .iter()
        .all(|group| app_state.workspace.tab_selection.contains(group));
    if all_selected {
        for group in groups {
            app_state.workspace.tab_selection.remove(group);
        }
    } else {
        for group in groups {
            app_state.workspace.tab_selection.insert(*group);
        }
    }
    cx.notify();
}

pub(crate) fn clear_tab_selection(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    if !app_state.workspace.tab_selection.is_empty() {
        app_state.workspace.tab_selection.clear();
        cx.notify();
    }
}

/// 按给定标签组与布局应用并排；由二级布局菜单点击后调用。
pub(crate) fn apply_tiling_layout_for(
    app_state: &mut AppState,
    groups: Vec<GroupId>,
    choice: TilingLayoutChoice,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let groups: Vec<GroupId> = groups
        .into_iter()
        .filter(|group| {
            app_state
                .workspace
                .groups
                .iter()
                .any(|item| item.id == *group)
        })
        .collect();
    if groups.len() < 2 {
        return;
    }
    if groups.len() > MAX_TILING_SLOTS {
        app_state.set_error(rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS).to_string());
        cx.notify();
        return;
    }
    let (rows, cols, auto) = match choice {
        TilingLayoutChoice::Auto => {
            let (rows, cols) = auto_tiling_grid(groups.len(), window);
            (rows, cols, true)
        }
        TilingLayoutChoice::Grid { rows, cols } => (rows, cols, false),
    };
    let touched: Vec<TilingId> = app_state
        .workspace
        .tiled_sets
        .iter()
        .filter(|tiled| groups.iter().any(|group| tiled.contains(*group)))
        .map(|tiled| tiled.id)
        .collect();
    match touched.as_slice() {
        [] => {
            let Some(tiled) = TiledTabs::new(groups, rows, cols, auto) else {
                return;
            };
            let focus = tiled.focused_group();
            app_state.workspace.tiled_sets.push(tiled);
            app_state.workspace.tab_selection.clear();
            app_state.prune_pane_split_states();
            activate_group_by_id(app_state, focus, window, cx);
        }
        [id] => {
            let Some(index) = app_state
                .workspace
                .tiled_sets
                .iter()
                .position(|tiled| tiled.id == *id)
            else {
                return;
            };
            if !groups
                .iter()
                .all(|group| app_state.workspace.tiled_sets[index].contains(*group))
            {
                return;
            }
            let focus = app_state.workspace.tiled_sets[index].focused_group();
            let Some(mut tiled) = TiledTabs::new(groups, rows, cols, auto) else {
                return;
            };
            tiled.id = *id;
            if let Some(focus) = focus
                && let Some(slot) = tiled.slot_of(focus)
            {
                tiled.focused = slot;
            }
            app_state.workspace.tiled_sets[index] = tiled;
            app_state.prune_pane_split_states();
            let focus = app_state.workspace.tiled_sets[index].focused_group();
            activate_group_by_id(app_state, focus, window, cx);
        }
        _ => return,
    }
    app_state.sync_broadcast_targets();
    cx.notify();
}

/// 激活某个标签组所在的标签；标签不存在时不做任何事。
fn activate_group_by_id(
    app_state: &mut AppState,
    group: Option<GroupId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(group) = group else {
        return;
    };
    crate::tiling::activate_group(app_state, group, window, cx);
}

/// 把一组独立标签加入指定并排组，返回第一个新标签组。
pub(crate) fn add_groups_to_tiling(
    app_state: &mut AppState,
    id: TilingId,
    groups: Vec<GroupId>,
    window: &Window,
) -> Option<GroupId> {
    let groups: Vec<GroupId> = groups
        .into_iter()
        .filter(|group| {
            app_state
                .workspace
                .groups
                .iter()
                .any(|item| item.id == *group)
        })
        .filter(|group| app_state.tiling_of_group(*group).is_none())
        .collect();
    if groups.is_empty() {
        return None;
    }
    let first_group = {
        let tiled = app_state
            .workspace
            .tiled_sets
            .iter_mut()
            .find(|tiled| tiled.id == id)?;
        let slot = tiled.add_groups(groups, window)?;
        tiled.group_at(slot)
    };
    let first_group = first_group?;
    app_state.prune_pane_split_states();
    app_state.sync_broadcast_targets();
    Some(first_group)
}

/// 把一个独立标签填进指定并排组的空槽位。
pub(crate) fn fill_tiling_slot(
    app_state: &mut AppState,
    id: TilingId,
    slot: usize,
    group: GroupId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if app_state.tiling_of_group(group).is_some() {
        return;
    }
    let filled = app_state
        .workspace
        .tiled_sets
        .iter_mut()
        .find(|tiled| tiled.id == id)
        .is_some_and(|tiled| tiled.fill_slot(slot, group));
    if !filled {
        return;
    }
    crate::tiling::activate_group(app_state, group, window, cx);
    app_state.sync_broadcast_targets();
}
