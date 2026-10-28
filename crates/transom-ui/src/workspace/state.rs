//! 工作区多标签与分屏网格核心状态容器。
//!
//! 设计原理（Why）：
//! - 实现初衷：从上帝对象 `AppShell` 中完整剥离标签页集合、分割树分组、并排网格与当前焦点追踪，
//!   消除上层组件与事件处理对全局大一统状态的过度依赖；
//! - 核心优势：
//!   1. 工作区生命周期与操作高内聚（标签增删、查找、当前活动标签状态判定）；
//!   2. 支持独立测试与方法下沉；
//!   3. `AppShell` 以字段形式持有本容器，标签与布局操作集中在其方法内，
//!      跨组件访问仍需显式写出 `workspace` 路径；
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

    /// 按窗格 id 查找标签。
    pub(crate) fn tab(&self, id: TabId) -> Option<&TerminalTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// 按窗格 id 可变查找标签。
    pub(crate) fn tab_mut(&mut self, id: TabId) -> Option<&mut TerminalTab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// 标签在集合中的下标；需要按下标同时访问其它工作区字段时使用。
    pub(crate) fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// 标签是否存在。
    pub(crate) fn has_tab(&self, id: TabId) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    /// 按分组 id 查找标签组。
    pub(crate) fn group(&self, id: GroupId) -> Option<&TabGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// 标签组在集合中的下标。
    pub(crate) fn group_index(&self, id: GroupId) -> Option<usize> {
        self.groups.iter().position(|group| group.id == id)
    }

    /// 标签所属分组的索引；标签不属于任何分组时返回 `None`。
    pub(crate) fn group_index_of(&self, tab: TabId) -> Option<usize> {
        self.groups.iter().position(|group| group.contains(tab))
    }

    /// 统一布局收敛入口：顺序固定为 并排网格 -> 分割拖拽状态 -> 多选。
    ///
    /// 网格阶段摘掉已消失的组、空槽补焦点，占用不足两个格子的网格整体解除；
    /// 反过来（先算分割存活再摘网格）会把已消失的节点当成仍存在，拖拽状态
    /// 留下死键。活动标签由调用方在删除标签后自行选择，故此处只校验布局引用。
    pub(crate) fn reconcile_layout(&mut self, active_group: Option<GroupId>) {
        self.reconcile_tilings_inner(active_group);
        self.reconcile_split_states_inner();
        self.reconcile_selection_inner();
        self.debug_assert_layout_refs();
    }

    fn reconcile_tilings_inner(&mut self, active_group: Option<GroupId>) {
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
    }

    /// 丢弃多选集合里已经关闭的标签组。
    pub(crate) fn reconcile_selection(&mut self) {
        self.reconcile_selection_inner();
        self.debug_assert_consistent();
    }

    fn reconcile_selection_inner(&mut self) {
        let groups = &self.groups;
        self.tab_selection
            .retain(|group| groups.iter().any(|item| item.id == *group));
    }

    /// 丢弃分割树上已消失节点的拖拽状态。
    pub(crate) fn reconcile_split_states(&mut self) {
        self.reconcile_split_states_inner();
        self.debug_assert_consistent();
    }

    fn reconcile_split_states_inner(&mut self) {
        let alive = alive_split_ids(&self.groups, &self.tiled_sets);
        self.pane_split_states.retain(|id, _| alive.contains(id));
    }

    /// 校验四张布局表之间的引用完整性；只在 debug 构建生效。
    #[cfg(debug_assertions)]
    fn debug_assert_layout_refs(&self) {
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
        let alive_splits = alive_split_ids(&self.groups, &self.tiled_sets);
        debug_assert!(
            self.pane_split_states
                .keys()
                .all(|id| alive_splits.contains(id)),
            "分割拖拽状态包含已消失的节点"
        );
    }

    /// 布局引用与活动标签的双重校验；活动标签允许由调用方在收敛后修复，
    /// 因此单步清理方法仍用它，统一收敛入口只用布局引用校验。
    #[cfg(debug_assertions)]
    fn debug_assert_consistent(&self) {
        self.debug_assert_layout_refs();
        if let Some(active) = self.active_tab {
            debug_assert!(
                self.tabs.iter().any(|tab| tab.id == active),
                "活动标签必须存在于标签集合"
            );
        }
    }

    #[cfg(not(debug_assertions))]
    fn debug_assert_layout_refs(&self) {}

    #[cfg(not(debug_assertions))]
    fn debug_assert_consistent(&self) {}
}

/// 当前仍被分割树引用的节点集合，供拖拽状态清理与 debug 校验共用。
fn alive_split_ids(groups: &[TabGroup], tiled_sets: &[TiledTabs]) -> HashSet<SplitId> {
    groups
        .iter()
        .flat_map(|group| group.root.split_ids())
        .chain(tiled_sets.iter().flat_map(|tiled| tiled.root.split_ids()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::{AuthMethod, Connection, Session, SessionId, SshConnection};

    /// 查询 API 与底层集合保持一致：标签、分组与归属关系都能按 id 命中。
    #[test]
    fn lookup_helpers_find_tabs_and_groups() {
        let mut workspace = WorkspaceState::new();
        let pane = TabId::new();
        let session = Session::new(
            "demo",
            Connection::Ssh(SshConnection {
                host: "example.com".into(),
                port: 22,
                username: "demo".into(),
                auth: AuthMethod::Password { save: false },
                options: Default::default(),
                proxy_jump: None,
            }),
        );
        let group = TabGroup::new(pane, session.id, 0);
        let group_id = group.id;
        workspace.groups.push(group);
        workspace.tabs.push(TerminalTab::pending(pane, session));

        assert!(workspace.tab(pane).is_some());
        assert_eq!(workspace.tab_index(pane), Some(0));
        assert!(workspace.has_tab(pane));
        assert!(workspace.tab_mut(pane).is_some());
        assert_eq!(workspace.group_index_of(pane), Some(0));
        assert!(workspace.group(group_id).is_some());
        assert_eq!(workspace.group_index(group_id), Some(0));

        let missing = TabId::new();
        assert!(workspace.tab(missing).is_none());
        assert!(!workspace.has_tab(missing));
        assert!(workspace.group_index_of(missing).is_none());
    }

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
        ws.reconcile_layout(None);

        assert!(
            ws.tiled_sets.is_empty(),
            "占用不足两个格子的并排网格必须解除"
        );
    }

    /// 统一收敛必须先摘网格再算分割存活：否则网格里的节点会被当成仍然存在，
    /// 拖拽状态留下死键（关闭并排标签时的 debug 断言缺陷）。
    #[test]
    fn reconcile_layout_prunes_grid_before_split_states() {
        let mut ws = WorkspaceState::new();
        let first = TabGroup::new(TabId::new(), SessionId::new(), 0);
        let second = TabGroup::new(TabId::new(), SessionId::new(), 1);
        let first_id = first.id;
        ws.groups.push(first);
        ws.groups.push(second);
        let second_id = ws.groups[1].id;
        let tiled = TiledTabs::new(vec![first_id, second_id], 1, 2, false).expect("合法 1x2");
        ws.tiled_sets.push(tiled);

        let grid_splits = alive_split_ids(&ws.groups, &ws.tiled_sets);
        assert!(!grid_splits.is_empty(), "1x2 网格应包含分割节点");

        ws.groups.remove(1);
        ws.reconcile_layout(None);

        assert!(ws.tiled_sets.is_empty(), "占用不足两格的网格必须解除");
        let remaining = alive_split_ids(&ws.groups, &ws.tiled_sets);
        assert!(
            grid_splits.is_disjoint(&remaining),
            "网格解除后其分割节点必须从存活集合消失"
        );
    }

    /// 活动标签在布局收敛时允许暂时失效（由调用方随后选择下一个），
    /// 收敛入口不得因此触发 debug 断言。
    #[test]
    fn reconcile_layout_tolerates_stale_active_tab() {
        let mut ws = WorkspaceState::new();
        ws.active_tab = Some(TabId::new());

        ws.reconcile_layout(None);

        assert!(ws.groups.is_empty());
    }
}
