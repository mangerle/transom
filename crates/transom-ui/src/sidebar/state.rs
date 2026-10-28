//! 会话侧边栏状态模型与行快照缓存管理。
//!
//! 设计原理（Why）：
//! 将侧边栏可见性、搜索框实体、行视图缓存与选择集从顶层 `AppShell` 抽离，
//! 形成自包含的侧栏状态单元，降低顶层单体复杂度并收敛侧栏交互边界。

use crate::app::SidebarRowsCache;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use std::collections::HashSet;
use transom_core::SessionId;

/// 会话侧边栏状态。
pub(crate) struct SidebarState {
    /// 侧栏显隐状态。
    pub(crate) visible: bool,
    /// 侧栏搜索框状态。
    pub(crate) search: Entity<InputState>,
    /// 侧栏行快照缓存：目录与搜索词不变时复用。
    pub(crate) rows_cache: Option<SidebarRowsCache>,
    /// 多选模式开关。
    pub(crate) selection_mode: bool,
    /// 已选中的会话 ID 集合。
    pub(crate) selected_sessions: HashSet<SessionId>,
    /// 当前单选高亮的会话 ID。
    pub(crate) tree_selection: Option<SessionId>,
}

impl SidebarState {
    /// 创建初始侧栏状态。
    pub(crate) fn new(visible: bool, search: Entity<InputState>) -> Self {
        Self {
            visible,
            search,
            rows_cache: None,
            selection_mode: false,
            selected_sessions: HashSet::with_capacity(8),
            tree_selection: None,
        }
    }

    /// 切换侧栏显隐。
    pub(crate) fn toggle_visible(&mut self) {
        self.visible = !self.visible;
    }

    /// 清理所有选中会话状态。
    pub(crate) fn clear_selection(&mut self) {
        self.selected_sessions.clear();
        self.tree_selection = None;
    }
}
