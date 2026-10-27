//! 工作区多标签与分屏网格核心状态容器。
//!
//! 设计原理（Why）：
//! - 实现初衷：从上帝对象 `AppShell` 中完整剥离标签页集合、分割树分组、并排网格与当前焦点追踪，
//!   消除上层组件与事件处理对全局大一统状态的过度依赖；
//! - 核心优势：
//!   1. 工作区生命周期与操作高内聚（标签增删、查找、当前活动标签状态判定）；
//!   2. 支持独立测试与方法下沉；
//!   3. `AppShell` 通过持有 `WorkspaceState` 并实现 `Deref`/`DerefMut`，既达成架构解耦，
//!      又保持外部已有调用的无缝兼容；
//! - 代价与局限：跨组件需要访问全局设置或持久化目录时，仍需上层 `AppShell` 协调。

use crate::app::{TabGroup, TerminalTab};
use crate::tiling::TiledTabs;
use gpui_kit::Entity;
use gpui_kit::component::resizable::ResizableState;
use std::collections::{HashMap, HashSet};
use transom_core::{GroupId, SplitId, TabId};

/// 工作区核心状态：聚合全部活动标签、分割树分组、并排平铺网格与焦点追踪。
pub struct WorkspaceState {
    pub(crate) tabs: Vec<TerminalTab>,
    pub(crate) groups: Vec<TabGroup>,
    pub(crate) tiled_sets: Vec<TiledTabs>,
    pub(crate) tab_selection: HashSet<GroupId>,
    pub(crate) pane_split_states: HashMap<SplitId, Entity<ResizableState>>,
    pub(crate) active_tab: Option<TabId>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl WorkspaceState {
    /// 构造空的工作区状态容器。
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            groups: Vec::new(),
            tiled_sets: Vec::new(),
            tab_selection: HashSet::new(),
            pane_split_states: HashMap::new(),
            active_tab: None,
        }
    }

    /// 获取当前活动标签 ID。
    pub(crate) fn active_tab(&self) -> Option<TabId> {
        self.active_tab
    }

    /// 设置当前活动标签。
    pub(crate) fn set_active_tab(&mut self, tab_id: Option<TabId>) {
        self.active_tab = tab_id;
    }

    /// 根据标签 ID 查找不可变标签引用。
    pub(crate) fn find_tab(&self, tab_id: TabId) -> Option<&TerminalTab> {
        self.tabs.iter().find(|t| t.id == tab_id)
    }

    /// 根据标签 ID 查找可变标签引用。
    pub(crate) fn find_tab_mut(&mut self, tab_id: TabId) -> Option<&mut TerminalTab> {
        self.tabs.iter_mut().find(|t| t.id == tab_id)
    }

    /// 根据标签组 ID 查找不可变分组引用。
    pub(crate) fn find_group(&self, group_id: GroupId) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.id == group_id)
    }

    /// 根据标签组 ID 查找可变分组引用。
    pub(crate) fn find_group_mut(&mut self, group_id: GroupId) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|g| g.id == group_id)
    }

    /// 查找包含指定窗格的分组。
    pub(crate) fn find_group_by_pane(&self, pane: TabId) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.contains(pane))
    }

    /// 查找包含指定窗格的可变分组。
    pub(crate) fn find_group_by_pane_mut(&mut self, pane: TabId) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|g| g.contains(pane))
    }

    /// 移除指定标签，并返回被移除的标签实体。
    pub(crate) fn remove_tab(&mut self, tab_id: TabId) -> Option<TerminalTab> {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == tab_id) {
            Some(self.tabs.remove(pos))
        } else {
            None
        }
    }

    /// 判断工作区是否为空（没有任何活动标签）。
    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// 当前活动标签数量。
    pub(crate) fn tab_count(&self) -> usize {
        self.tabs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_state_lifecycle_and_helpers() {
        let mut ws = WorkspaceState::new();
        assert!(ws.is_empty());
        assert_eq!(ws.tab_count(), 0);
        assert_eq!(ws.active_tab(), None);

        let tab_id = TabId::new();
        ws.set_active_tab(Some(tab_id));
        assert_eq!(ws.active_tab(), Some(tab_id));
        assert!(ws.find_tab(tab_id).is_none());
        assert!(ws.find_tab_mut(tab_id).is_none());
        assert!(ws.remove_tab(tab_id).is_none());

        let group_id = GroupId::new();
        assert!(ws.find_group(group_id).is_none());
        assert!(ws.find_group_mut(group_id).is_none());
        assert!(ws.find_group_by_pane(tab_id).is_none());
        assert!(ws.find_group_by_pane_mut(tab_id).is_none());
    }
}
