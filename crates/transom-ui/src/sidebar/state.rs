//! 左侧槽位状态模型与行快照缓存管理。
//!
//! 设计原理（Why）：
//! 将左侧槽位当前视图、可见性、搜索框实体、行视图缓存与选择集从顶层
//! `AppShell` 抽离，形成自包含的侧栏状态单元，降低顶层单体复杂度并收敛
//! 侧栏交互边界。槽位视图使用枚举而非多个互斥布尔位，保证任意时刻只有
//! 一个面板处于激活状态。

use super::SidebarRow;
use super::SidebarSlotSync;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use std::collections::HashSet;
use std::rc::Rc;
use transom_core::{SessionId, TabPosition};

/// 左侧槽位当前承载的主视图。
///
/// 设计原理（Why）：会话树、纵向标签、容器和 Git 共用同一列物理空间，
/// 用枚举表达互斥关系可以让“同时打开两个面板”在类型层面不可表示，避免
/// 渲染优先级掩盖状态错误。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SidebarSection {
    /// 会话目录树。
    #[default]
    Sessions,
    /// 纵向终端标签列表。
    Tabs,
    /// 容器连接与引擎列表。
    Containers,
    /// Git 项目列表。
    Git,
    /// 云同步状态、冲突与历史。
    CloudSync,
}

impl SidebarSection {
    /// 按标签位置解析槽位可展示的实际视图。
    ///
    /// `Tabs` 只在标签位于左侧时有效；配置切到顶部后立刻回退到会话树。
    /// 该解析是渲染与活动栏高亮的共同口径，避免配置变化的一帧里显示会话树
    /// 却仍高亮纵向标签入口。
    pub(crate) fn resolved_for_tab_position(self, tab_position: TabPosition) -> Self {
        if self == Self::Tabs && tab_position != TabPosition::Left {
            Self::Sessions
        } else {
            self
        }
    }
}

/// 左侧槽位的显隐与当前视图。
///
/// 显隐与视图分开保存：快捷键收起侧栏后再次展开，仍应回到收起前的视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SidebarNavigation {
    visible: bool,
    section: SidebarSection,
}

impl SidebarNavigation {
    /// 使用启动时的显隐状态与默认视图创建导航状态。
    fn new(visible: bool, section: SidebarSection) -> Self {
        Self { visible, section }
    }

    /// 当前是否正在展示指定视图。
    fn is_showing(&self, section: SidebarSection) -> bool {
        self.visible && self.section == section
    }

    /// 切换指定视图，返回切换后槽位是否展开。
    ///
    /// 点击当前已展示视图时收起槽位但保留当前视图；点击其他视图时直接
    /// 切换并展开，避免旧视图标记残留。
    fn toggle_section(&mut self, section: SidebarSection) -> bool {
        if self.is_showing(section) {
            self.visible = false;
            false
        } else {
            self.section = section;
            self.visible = true;
            true
        }
    }

    /// 切换到指定视图并确保槽位展开。
    fn show_section(&mut self, section: SidebarSection) {
        self.section = section;
        self.visible = true;
    }

    /// 切换槽位显隐，不改变当前视图。
    fn toggle_visible(&mut self) {
        self.visible = !self.visible;
    }

    /// 仅更新当前视图，不改变显隐状态。
    fn select_section(&mut self, section: SidebarSection) {
        self.section = section;
    }
}

/// 侧栏行快照缓存：目录版本与搜索词不变时直接复用已构建的行。
pub(crate) struct SidebarRowsCache {
    pub(crate) revision: u64,
    pub(crate) query: String,
    pub(crate) rows: Rc<Vec<SidebarRow>>,
}

/// 会话侧边栏状态。
pub(crate) struct SidebarState {
    /// 左侧槽位的显隐与当前视图。
    navigation: SidebarNavigation,
    /// 侧栏在可拖拽分组里的槽位宽度同步状态：收起/展开与停靠侧切换后据此恢复。
    pub(crate) slot: SidebarSlotSync,
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
    pub(crate) fn new(visible: bool, section: SidebarSection, search: Entity<InputState>) -> Self {
        Self {
            navigation: SidebarNavigation::new(visible, section),
            slot: SidebarSlotSync::new(visible),
            search,
            rows_cache: None,
            selection_mode: false,
            selected_sessions: HashSet::with_capacity(8),
            tree_selection: None,
        }
    }

    /// 切换侧栏显隐。
    pub(crate) fn toggle_visible(&mut self) {
        self.navigation.toggle_visible();
    }

    /// 当前槽位是否可见。
    pub(crate) fn visible(&self) -> bool {
        self.navigation.visible
    }

    /// 当前槽位承载的视图。
    pub(crate) fn section(&self) -> SidebarSection {
        self.navigation.section
    }

    /// 指定视图当前是否正在展示。
    pub(crate) fn is_showing(&self, section: SidebarSection) -> bool {
        self.navigation.is_showing(section)
    }

    /// 切换指定视图，返回切换后槽位是否展开。
    pub(crate) fn toggle_section(&mut self, section: SidebarSection) -> bool {
        self.navigation.toggle_section(section)
    }

    /// 切换到指定视图并确保槽位展开。
    pub(crate) fn show_section(&mut self, section: SidebarSection) {
        self.navigation.show_section(section);
    }

    /// 仅更新当前视图，不改变显隐状态。
    pub(crate) fn select_section(&mut self, section: SidebarSection) {
        self.navigation.select_section(section);
    }

    /// 清理所有选中会话状态。
    pub(crate) fn clear_selection(&mut self) {
        self.selected_sessions.clear();
        self.tree_selection = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{SidebarNavigation, SidebarSection};
    use transom_core::TabPosition;

    #[test]
    fn switching_sections_keeps_exactly_one_active_view() {
        let mut navigation = SidebarNavigation::new(true, SidebarSection::Git);

        assert!(navigation.toggle_section(SidebarSection::Sessions));
        assert!(navigation.visible);
        assert_eq!(navigation.section, SidebarSection::Sessions);
        assert!(navigation.is_showing(SidebarSection::Sessions));
        assert!(!navigation.is_showing(SidebarSection::Git));
    }

    #[test]
    fn clicking_active_section_hides_without_losing_selection() {
        let mut navigation = SidebarNavigation::new(true, SidebarSection::Sessions);

        assert!(!navigation.toggle_section(SidebarSection::Sessions));
        assert!(!navigation.visible);
        assert_eq!(navigation.section, SidebarSection::Sessions);

        assert!(navigation.toggle_section(SidebarSection::Sessions));
        assert!(navigation.visible);
        assert!(navigation.is_showing(SidebarSection::Sessions));
    }

    #[test]
    fn tabs_section_falls_back_to_sessions_when_tabs_move_to_top() {
        assert_eq!(
            SidebarSection::Tabs.resolved_for_tab_position(TabPosition::Top),
            SidebarSection::Sessions
        );
        assert_eq!(
            SidebarSection::Tabs.resolved_for_tab_position(TabPosition::Left),
            SidebarSection::Tabs
        );
        assert_eq!(
            SidebarSection::Git.resolved_for_tab_position(TabPosition::Top),
            SidebarSection::Git
        );
    }

    #[test]
    fn toggling_visibility_preserves_selected_section() {
        let mut navigation = SidebarNavigation::new(true, SidebarSection::Containers);

        navigation.toggle_visible();
        assert!(!navigation.visible);
        assert_eq!(navigation.section, SidebarSection::Containers);

        navigation.toggle_visible();
        assert!(navigation.is_showing(SidebarSection::Containers));
    }
}
