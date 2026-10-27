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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_state_lifecycle() {
        let ws = WorkspaceState::new();
        assert!(ws.tabs.is_empty());
        assert!(ws.groups.is_empty());
        assert_eq!(ws.active_tab, None);
    }
}
