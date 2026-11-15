//! WorkspaceState 的多标签并排查询与槽位恢复辅助。

use super::strip::build_strip_entries;
use super::tabs::TiledTabs;
use super::types::{StripEntry, TilingId};
use crate::app::AppState;
use transom_core::{GroupId, SavedTabTiling};

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
    pub(super) fn previous_strip_entry(&self, target: GroupId) -> Option<StripEntry> {
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
