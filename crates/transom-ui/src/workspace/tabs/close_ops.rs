//! 标签窗格摘除与可见性判定（纯工作区状态操作）。

use crate::app::AppState;
use crate::pane_tree::RemoveOutcome;
use transom_core::TabId;

impl AppState {
    /// 当前活动标签所属分组的索引；没有活动标签时为 `None`。
    pub(crate) fn active_group_index(&self) -> Option<usize> {
        self.workspace
            .active_tab
            .and_then(|tab_id| self.workspace.group_index_for_tab(tab_id))
    }

    /// 该窗格此刻是否正被渲染：属于可见标签组、在分割树里，且没被"单窗格最大化"挡在后面。
    pub(crate) fn is_rendered_pane(&self, tab_id: TabId) -> bool {
        let Some(group_index) = self.workspace.group_index_for_tab(tab_id) else {
            return false;
        };
        let group = &self.workspace.groups[group_index];
        let visible = match self.active_tiling() {
            Some(tiled) => tiled.contains(group.id),
            None => self.active_group_index() == Some(group_index),
        };
        if !visible {
            return false;
        }
        match group.zoomed {
            Some(zoomed) => zoomed == tab_id,
            None => group.root.contains(tab_id),
        }
    }

    /// 从所属标签组的分割树里摘除一个窗格，返回应接管的焦点窗格。
    pub(crate) fn detach_pane_from_group(&mut self, tab_id: TabId) -> Option<TabId> {
        let group_index = self.workspace.group_index_for_tab(tab_id)?;
        let was_active = self.workspace.active_tab == Some(tab_id);
        let replacement = match self.workspace.groups[group_index].root.remove_leaf(tab_id) {
            RemoveOutcome::Empty => {
                self.workspace.groups.remove(group_index);
                if was_active {
                    self.workspace
                        .groups
                        .get(group_index.saturating_sub(1))
                        .or_else(|| self.workspace.groups.first())
                        .map(|group| group.active_pane)
                } else {
                    None
                }
            }
            RemoveOutcome::Removed { next_focus } => {
                let group = &mut self.workspace.groups[group_index];
                if group.active_pane == tab_id {
                    group.active_pane = next_focus;
                }
                if group.zoomed == Some(tab_id) {
                    group.zoomed = None;
                }
                if was_active { Some(next_focus) } else { None }
            }
        };
        self.prune_pane_split_states();
        replacement
    }

    /// 移除指定标签并按优先级回退焦点窗格。
    pub(crate) fn remove_tab(&mut self, tab_id: TabId) {
        let was_active = self.workspace.active_tab == Some(tab_id);
        let group_id = self
            .workspace
            .group_index_for_tab(tab_id)
            .map(|index| self.workspace.groups[index].id);
        let tiled_focus = group_id
            .and_then(|group| self.tiling_of_group(group))
            .and_then(|tiled| tiled.focused_group());
        self.workspace.tabs.retain(|tab| tab.id != tab_id);
        let replacement = self.detach_pane_from_group(tab_id);
        self.prune_tilings();
        if was_active {
            self.workspace.active_tab = self.fallback_active_pane(replacement, tiled_focus);
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
    }
}
