//! AppShell 并排状态增删查改与条目装配实现。

use super::layout::auto_tiling_grid;
use super::strip::build_strip_entries;
use super::tabs::TiledTabs;
use super::types::{MAX_TILING_SLOTS, StripEntry, TilingId, TilingLayoutChoice};
use crate::app::{AppShell, AppState};
use crate::pane_tree::PaneDirection;
use crate::sftp::PendingEditorClose;
use gpui_kit::*;
use transom_core::{GroupId, SavedTabTiling, SplitAxis, TabId};

impl AppState {
    /// 当前焦点标签所在的标签组；没有活动标签时为 `None`。
    pub(crate) fn active_group_id(&self) -> Option<GroupId> {
        self.workspace
            .active_tab
            .and_then(|tab| self.workspace.group_index_for_tab(tab))
            .map(|index| self.workspace.groups[index].id)
    }

    /// 当前活动标签所属的并排组；活动标签独立显示时为 `None`。
    pub(crate) fn active_tiling(&self) -> Option<&TiledTabs> {
        let active = self.active_group_id()?;
        self.tiling_of_group(active)
    }

    /// 某个标签组所在的并排组；未并排时为 `None`。
    pub(crate) fn tiling_of_group(&self, group: GroupId) -> Option<&TiledTabs> {
        self.workspace
            .tiled_sets
            .iter()
            .find(|tiled| tiled.contains(group))
    }

    /// 按并排组身份查找。
    pub(crate) fn tiling_by_id(&self, id: TilingId) -> Option<&TiledTabs> {
        self.workspace
            .tiled_sets
            .iter()
            .find(|tiled| tiled.id == id)
    }

    /// 标签栏的可见条目：每个并排组占一条，其余标签各占一条。
    pub(crate) fn strip_entries(&self) -> Vec<StripEntry> {
        let groups: Vec<GroupId> = self.workspace.groups.iter().map(|group| group.id).collect();
        build_strip_entries(&groups, &self.workspace.tiled_sets)
    }

    /// 条目是否包含指定标签组；并排条目按整组成员判断。
    pub(crate) fn entry_contains(&self, entry: StripEntry, group: GroupId) -> bool {
        match entry {
            StripEntry::Single(item) => item == group,
            StripEntry::Tiled { id, .. } => self
                .tiling_by_id(id)
                .is_some_and(|tiled| tiled.contains(group)),
        }
    }

    /// 标签栏里目标标签左边那一条（首位回绕到末位）。
    fn previous_strip_entry(&self, target: GroupId) -> Option<StripEntry> {
        let entries = self.strip_entries();
        let position = entries
            .iter()
            .position(|entry| self.entry_contains(*entry, target))?;
        if position == 0 {
            entries.last().copied()
        } else {
            entries.get(position - 1).copied()
        }
    }

    /// 同步活动标签与所在并排组的焦点槽，不改变并排关系本身。
    pub(crate) fn present_active_group(&mut self) {
        if let Some(active) = self.active_group_id()
            && let Some(tiled) = self
                .workspace
                .tiled_sets
                .iter_mut()
                .find(|tiled| tiled.contains(active))
            && let Some(slot) = tiled.slot_of(active)
        {
            tiled.focused = slot;
        }
        self.sync_broadcast_targets();
    }

    /// 校验所有并排绑定：摘掉已消失的标签组，有效槽位少于两个时解除该组，
    /// 随后按新的分割树清理拖拽状态与多选。
    pub(crate) fn prune_tilings(&mut self) {
        let active = self.active_group_id();
        self.workspace.reconcile_layout(active);
        self.sync_broadcast_targets();
    }

    /// 按持久化快照恢复一张并排网格，返回它的焦点标签。
    pub(crate) fn restore_tab_tiling(
        &mut self,
        saved: &SavedTabTiling,
        restored: &[Option<GroupId>],
    ) -> Option<GroupId> {
        if saved.rows == 0
            || saved.cols == 0
            || saved.slots.len() != saved.rows.saturating_mul(saved.cols)
        {
            return None;
        }
        let slots: Vec<Option<GroupId>> = saved
            .slots
            .iter()
            .map(|index| index.and_then(|index| restored.get(index).copied().flatten()))
            .collect();
        let tiled =
            TiledTabs::from_saved_slots(slots, saved.rows, saved.cols, saved.focused, saved.auto)?;
        if tiled
            .occupied_groups()
            .iter()
            .any(|group| self.tiling_of_group(*group).is_some())
        {
            return None;
        }
        let focus = tiled.focused_group();
        self.workspace.tiled_sets.push(tiled);
        self.prune_pane_split_states();
        focus
    }

    /// 丢弃已经关闭的标签组，避免多选集合里残留死 ID。
    pub(crate) fn prune_tab_selection(&mut self) {
        self.workspace.reconcile_selection();
    }

    /// 可填进空槽的标签组：还没有属于任何并排组的独立标签。
    pub(crate) fn available_tiling_groups(&self) -> Vec<GroupId> {
        self.workspace
            .groups
            .iter()
            .filter(|group| self.tiling_of_group(group.id).is_none())
            .map(|group| group.id)
            .collect()
    }
}

/// 快速并排：把右键的标签和它在标签栏里左边那一条并到一起。
pub(crate) fn tile_tab(
    shell: &mut AppShell,
    target: GroupId,
    axis: SplitAxis,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(target_pane) = shell
        .state
        .workspace
        .group(target)
        .map(|group| group.active_pane)
    else {
        return;
    };
    if shell.state.tiling_of_group(target).is_some() {
        crate::app::tiling::activate_group(shell, target, window, cx);
        return;
    }
    let Some(previous) = shell.state.previous_strip_entry(target) else {
        return;
    };
    match previous {
        StripEntry::Tiled { id, .. } => {
            if let Some(group) = crate::app::tiling::add_groups_to_tiling(shell, id, vec![target], window) {
                crate::app::tiling::activate_group(shell, group, window, cx);
            } else {
                shell.state
                    .set_error(rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS));
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
            shell.state.workspace.tiled_sets.push(tiled);
            shell.state.prune_pane_split_states();
            shell.activate_tab(target_pane, window, cx);
        }
        StripEntry::Single(_) => {
            shell.activate_tab(target_pane, window, cx);
        }
    }
}

/// 关闭指定并排组里的全部标签（合并条目关闭菜单的第二项）。
pub(crate) fn close_tiling_group(
    shell: &mut AppShell,
    id: TilingId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(groups) = shell
        .state
        .tiling_by_id(id)
        .map(|tiled| tiled.occupied_groups())
    else {
        return;
    };
    crate::app::tiling::close_tiling_groups(shell, groups, window, cx);
}

/// 关闭指定的一组并排标签：先统一过脏编辑闸门，再逐个关闭标签组。
pub(crate) fn close_tiling_groups(
    shell: &mut AppShell,
    groups: Vec<GroupId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let panes: Vec<TabId> = groups
        .iter()
        .filter_map(|group_id| {
            shell.state
                .workspace
                .groups
                .iter()
                .find(|group| group.id == *group_id)
        })
        .flat_map(|group| group.pane_ids())
        .collect();
    if crate::editor::block_close_for_dirty_editors(shell, 
        &panes,
        PendingEditorClose::Tiling(groups.clone()),
        cx,
    ) {
        return;
    }
    for group_id in groups {
        if shell
            .state
            .workspace
            .groups
            .iter()
            .any(|group| group.id == group_id)
        {
            shell.close_group(group_id, window, cx);
        }
    }
}

/// 退出指定并排组，只改变显示方式：组内标签都保持打开与连接。
pub(crate) fn untile_tiling(shell: &mut AppShell, id: TilingId, cx: &mut Context<AppShell>) {
    let before = shell.state.workspace.tiled_sets.len();
    shell.state
        .workspace
        .tiled_sets
        .retain(|tiled| tiled.id != id);
    if shell.state.workspace.tiled_sets.len() != before {
        shell.state.prune_pane_split_states();
        shell.state.sync_broadcast_targets();
        cx.notify();
    }
}

/// 激活某个标签组（通常来自并排格标题栏或点击格内容）。
pub(crate) fn activate_group(
    shell: &mut AppShell,
    group: GroupId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(index) = shell.state.workspace.group_index(group) else {
        return;
    };
    let pane = shell.state.workspace.groups[index].active_pane;
    shell.activate_tab(pane, window, cx);
}

/// 窗格方向键在组内已无邻居时，沿网格方向跨到相邻槽位。
pub(crate) fn focus_neighbor_tile(
    shell: &mut AppShell,
    direction: PaneDirection,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(tiled) = shell.state.active_tiling().cloned() else {
        return;
    };
    let Some(active) = shell.state.active_group_id() else {
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
    crate::app::tiling::activate_group(shell, group, window, cx);
}

/// Ctrl/Cmd+点击标签：整组加入或移出多选集合。
pub(crate) fn toggle_tab_selection(shell: &mut AppShell, groups: &[GroupId], cx: &mut Context<AppShell>) {
    if groups.is_empty() {
        return;
    }
    let all_selected = groups
        .iter()
        .all(|group| shell.state.workspace.tab_selection.contains(group));
    if all_selected {
        for group in groups {
            shell.state.workspace.tab_selection.remove(group);
        }
    } else {
        for group in groups {
            shell.state.workspace.tab_selection.insert(*group);
        }
    }
    cx.notify();
}

pub(crate) fn clear_tab_selection(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    if !shell.state.workspace.tab_selection.is_empty() {
        shell.state.workspace.tab_selection.clear();
        cx.notify();
    }
}

/// 按给定标签组与布局应用并排；由二级布局菜单点击后调用。
pub(crate) fn apply_tiling_layout_for(
    shell: &mut AppShell,
    groups: Vec<GroupId>,
    choice: TilingLayoutChoice,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let groups: Vec<GroupId> = groups
        .into_iter()
        .filter(|group| {
            shell.state
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
        shell.state
            .set_error(rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS).to_string());
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
    let touched: Vec<TilingId> = shell
        .state
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
            shell.state.workspace.tiled_sets.push(tiled);
            shell.state.workspace.tab_selection.clear();
            shell.state.prune_pane_split_states();
            activate_group_by_id(shell, focus, window, cx);
        }
        [id] => {
            let Some(index) = shell
                .state
                .workspace
                .tiled_sets
                .iter()
                .position(|tiled| tiled.id == *id)
            else {
                return;
            };
            if !groups
                .iter()
                .all(|group| shell.state.workspace.tiled_sets[index].contains(*group))
            {
                return;
            }
            let focus = shell.state.workspace.tiled_sets[index].focused_group();
            let Some(mut tiled) = TiledTabs::new(groups, rows, cols, auto) else {
                return;
            };
            tiled.id = *id;
            if let Some(focus) = focus
                && let Some(slot) = tiled.slot_of(focus)
            {
                tiled.focused = slot;
            }
            shell.state.workspace.tiled_sets[index] = tiled;
            shell.state.prune_pane_split_states();
            let focus = shell.state.workspace.tiled_sets[index].focused_group();
            activate_group_by_id(shell, focus, window, cx);
        }
        _ => return,
    }
    shell.state.sync_broadcast_targets();
    cx.notify();
}

/// 激活某个标签组所在的标签；标签不存在时不做任何事。
fn activate_group_by_id(
    shell: &mut AppShell,
    group: Option<GroupId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(group) = group else {
        return;
    };
    crate::app::tiling::activate_group(shell, group, window, cx);
}

/// 把一组独立标签加入指定并排组，返回第一个新标签组。
pub(crate) fn add_groups_to_tiling(
    shell: &mut AppShell,
    id: TilingId,
    groups: Vec<GroupId>,
    window: &Window,
) -> Option<GroupId> {
    let groups: Vec<GroupId> = groups
        .into_iter()
        .filter(|group| {
            shell.state
                .workspace
                .groups
                .iter()
                .any(|item| item.id == *group)
        })
        .filter(|group| shell.state.tiling_of_group(*group).is_none())
        .collect();
    if groups.is_empty() {
        return None;
    }
    let first_group = {
        let tiled = shell
            .state
            .workspace
            .tiled_sets
            .iter_mut()
            .find(|tiled| tiled.id == id)?;
        let slot = tiled.add_groups(groups, window)?;
        tiled.group_at(slot)
    };
    let first_group = first_group?;
    shell.state.prune_pane_split_states();
    shell.state.sync_broadcast_targets();
    Some(first_group)
}

/// 把一个独立标签填进指定并排组的空槽位。
pub(crate) fn fill_tiling_slot(
    shell: &mut AppShell,
    id: TilingId,
    slot: usize,
    group: GroupId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if shell.state.tiling_of_group(group).is_some() {
        return;
    }
    let filled = shell
        .state
        .workspace
        .tiled_sets
        .iter_mut()
        .find(|tiled| tiled.id == id)
        .is_some_and(|tiled| tiled.fill_slot(slot, group));
    if !filled {
        return;
    }
    crate::app::tiling::activate_group(shell, group, window, cx);
    shell.state.sync_broadcast_targets();
}
