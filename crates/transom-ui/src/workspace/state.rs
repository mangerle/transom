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

    /// 收敛并排网格与标签组的关系：摘掉已消失的组，空槽补焦点，
    /// 占用不足两个格子的网格整体解除。
    ///
    /// 设计原理（Why）：并排、多选与拖拽状态三张辅助表都引用标签组与分割
    /// 节点，清理逻辑原先散落在三处 `prune_*` 里，新增一条布局变更路径就
    /// 容易漏掉其中一张表。收敛到状态容器后，清理规则与数据放在一起，
    /// 调用方只表达"布局变了"；debug 构建在每次收敛后校验引用完整性。
    ///
    /// 代价与局限：三个入口仍分开暴露——`reconcile_tilings` 会解除占用不足
    /// 的网格，不能在"网格尚未填满"的构造中途调用；多选与分割状态则可以
    /// 在任意布局变更后单独收敛。
    pub(crate) fn reconcile_tilings(&mut self, active_group: Option<GroupId>) {
        let alive: HashSet<GroupId> = self.groups.iter().map(|group| group.id).collect();
        self.tiled_sets.retain_mut(|tiled| {
            let dead: Vec<GroupId> = tiled
                .occupied_groups()
                .into_iter()
                .filter(|group| !alive.contains(group))
                .collect();
            for group in dead {
                tiled.remove_group(group);
            }
            if tiled.occupied_count() < 2 {
                return false;
            }
            if let Some(active) = active_group
                && let Some(slot) = tiled.slot_of(active)
            {
                tiled.focused = slot;
            }
            tiled.ensure_focus();
            true
        });
        self.debug_assert_consistent();
    }

    /// 丢弃多选集合里已经关闭的标签组。
    pub(crate) fn reconcile_selection(&mut self) {
        let groups = &self.groups;
        self.tab_selection
            .retain(|group| groups.iter().any(|item| item.id == *group));
        self.debug_assert_consistent();
    }

    /// 丢弃分割树上已消失节点的拖拽状态。
    pub(crate) fn reconcile_split_states(&mut self) {
        let alive: Vec<SplitId> = self
            .groups
            .iter()
            .flat_map(|group| group.root.split_ids())
            .chain(
                self.tiled_sets
                    .iter()
                    .flat_map(|tiled| tiled.root.split_ids()),
            )
            .collect();
        self.pane_split_states.retain(|id, _| alive.contains(id));
        self.debug_assert_consistent();
    }

    /// 校验五张表之间的引用完整性；只在 debug 构建生效。
    #[cfg(debug_assertions)]
    fn debug_assert_consistent(&self) {
        debug_assert!(
            self.groups.iter().all(|group| group.pane_count() >= 1),
            "标签组不得为空"
        );
        let mut seen = HashSet::with_capacity(self.tiled_sets.len());
        for tiled in &self.tiled_sets {
            for group in tiled.occupied_groups() {
                debug_assert!(
                    self.groups.iter().any(|item| item.id == group),
                    "并排组引用了不存在的标签组"
                );
                debug_assert!(seen.insert(group), "同一标签组不得出现在多个并排组");
            }
        }
        debug_assert!(
            self.tab_selection
                .iter()
                .all(|group| self.groups.iter().any(|item| item.id == *group)),
            "多选集合包含已关闭的标签组"
        );
        let alive_splits: Vec<SplitId> = self
            .groups
            .iter()
            .flat_map(|group| group.root.split_ids())
            .chain(
                self.tiled_sets
                    .iter()
                    .flat_map(|tiled| tiled.root.split_ids()),
            )
            .collect();
        debug_assert!(
            self.pane_split_states
                .keys()
                .all(|id| alive_splits.contains(id)),
            "分割拖拽状态包含已消失的节点"
        );
        if let Some(active) = self.active_tab {
            debug_assert!(
                self.tabs.iter().any(|tab| tab.id == active),
                "活动标签必须存在于标签集合"
            );
        }
    }

    #[cfg(not(debug_assertions))]
    fn debug_assert_consistent(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::SessionId;

    #[test]
    fn workspace_state_lifecycle() {
        let ws = WorkspaceState::new();
        assert!(ws.tabs.is_empty());
        assert!(ws.groups.is_empty());
        assert_eq!(ws.active_tab, None);
    }

    #[test]
    fn reconcile_selection_drops_dead_groups() {
        let mut ws = WorkspaceState::new();
        let group = TabGroup::new(TabId::new(), SessionId::new(), 0);
        let alive = group.id;
        ws.groups.push(group);
        ws.tab_selection.insert(alive);
        ws.tab_selection.insert(GroupId::new());

        ws.reconcile_selection();

        assert_eq!(ws.tab_selection.len(), 1);
        assert!(ws.tab_selection.contains(&alive), "现存标签组必须保留");
    }

    #[test]
    fn reconcile_tilings_drops_grid_below_two_groups() {
        let mut ws = WorkspaceState::new();
        let first = TabGroup::new(TabId::new(), SessionId::new(), 0);
        let second = TabGroup::new(TabId::new(), SessionId::new(), 1);
        let first_id = first.id;
        ws.groups.push(first);
        ws.groups.push(second);
        let second_id = ws.groups[1].id;
        let tiled = TiledTabs::new(vec![first_id, second_id], 1, 2, false).expect("合法 1x2");
        ws.tiled_sets.push(tiled);

        ws.groups.remove(1);
        ws.reconcile_tilings(None);

        assert!(
            ws.tiled_sets.is_empty(),
            "占用不足两个格子的并排网格必须解除"
        );
    }
}
