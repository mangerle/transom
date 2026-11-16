//! 标签内分屏：窗格的切分、方向翻转、焦点移动、最大化与摘出为独立标签。
//!
//! 设计原理（Why）：分屏状态完全属于工作区，本模块只借用工作区与会话目录，
//! 不接收 `AppShell`。窗口聚焦、建立连接和跨并排组导航由外壳键盘入口协调。

use crate::app::TerminalTab;
use crate::pane_tree::{MAX_PANES, PaneDirection, RemoveOutcome};
use crate::workspace::WorkspaceState;
use transom_core::{Connection, Session, SessionCatalog, SplitAxis, TabId};

/// 分屏状态操作的窄上下文。
pub(crate) struct SplitOps<'a> {
    workspace: &'a mut WorkspaceState,
    catalog: &'a SessionCatalog,
}

/// 分屏切分成功后需要外壳补建的连接。
pub(crate) struct SplitPanePlan {
    pub(crate) pane_id: TabId,
    pub(crate) session: Session,
}

/// 焦点移动结果：组内窗格或跨并排格。
pub(crate) enum SplitFocusPlan {
    Pane(TabId),
    NeighborTiling(PaneDirection),
}

impl<'a> SplitOps<'a> {
    /// 从应用状态拆借分屏域能力。
    pub(crate) fn new(state: &'a mut crate::app::AppState) -> Self {
        Self {
            workspace: &mut state.workspace,
            catalog: &state.catalog,
        }
    }

    /// 把焦点窗格沿 `axis` 切一半，新窗格复用当前会话配置另开一条连接。
    ///
    /// 只切焦点窗格本身：兄弟窗格及其已拖出的比例原样保留，这是分割树相对
    /// 扁平列表的核心收益。串口设备端口独占，不能被第二个窗格重复打开。
    pub(crate) fn split_active_pane(
        &mut self,
        axis: SplitAxis,
    ) -> Result<Option<SplitPanePlan>, String> {
        let Some(active_id) = self.workspace.active_tab else {
            return Ok(None);
        };
        let Some(group_index) = self.workspace.group_index_for_tab(active_id) else {
            return Ok(None);
        };
        let Some(session) = self.session_for_tab(active_id) else {
            return Ok(None);
        };
        if matches!(session.connection, Connection::Serial(_)) {
            return Err(rust_i18n::t!("error.serial_no_split").to_string());
        }
        if self.workspace.groups[group_index].pane_count() >= MAX_PANES {
            return Err(rust_i18n::t!("error.too_many_panes", max = MAX_PANES).to_string());
        }

        let pane_id = TabId::new();
        if !self.workspace.groups[group_index]
            .root
            .split_leaf(active_id, axis, pane_id)
        {
            return Ok(None);
        }
        let group = &mut self.workspace.groups[group_index];
        group.active_pane = pane_id;
        // 切分后新窗格接管焦点，若仍停留在原窗格的最大化上会把它藏起来。
        group.zoomed = None;
        self.workspace
            .tabs
            .push(TerminalTab::pending(pane_id, session.clone()));
        self.workspace.active_tab = Some(pane_id);
        Ok(Some(SplitPanePlan { pane_id, session }))
    }

    /// 翻转焦点窗格所在那一刀的方向，外层结构与兄弟窗格尺寸不受影响。
    pub(crate) fn toggle_split_axis(&mut self) -> bool {
        let Some(group_index) = self.active_group_index() else {
            return false;
        };
        let active_pane = self.workspace.groups[group_index].active_pane;
        // 单窗格没有父分割节点，toggle_axis 自然返回 false。
        if !self.workspace.groups[group_index]
            .root
            .toggle_axis(active_pane)
        {
            return false;
        }
        self.workspace.groups[group_index].zoomed = None;
        true
    }

    /// 在“只看这个窗格”与“按分割树铺满”之间切换。
    ///
    /// 最大化只改渲染视图、不动分割树，因此恢复分屏时布局与比例原样回来。
    pub(crate) fn toggle_zoom_pane(&mut self, pane: TabId) -> bool {
        let Some(group_index) = self.workspace.group_index_for_tab(pane) else {
            return false;
        };
        if self.workspace.groups[group_index].pane_count() < 2 {
            return false;
        }
        let group = &mut self.workspace.groups[group_index];
        group.zoomed = if group.zoomed == Some(pane) {
            None
        } else {
            Some(pane)
        };
        true
    }

    /// 把窗格摘出为独立标签组：连接、终端视图与滚动缓冲原地保留。
    pub(crate) fn detach_pane_to_tab(&mut self, tab_id: TabId) -> Option<TabId> {
        let group_index = self.workspace.group_index_for_tab(tab_id)?;
        if self.workspace.groups[group_index].pane_count() < 2 {
            return None;
        }
        let RemoveOutcome::Removed { next_focus } =
            self.workspace.groups[group_index].root.remove_leaf(tab_id)
        else {
            return None;
        };
        let group = &mut self.workspace.groups[group_index];
        if group.active_pane == tab_id {
            group.active_pane = next_focus;
        }
        if group.zoomed == Some(tab_id) {
            group.zoomed = None;
        }
        // 摘出后的标签仍属于原会话：SSH 按会话序号继续排；本地终端每次都是
        // 新 Session，改按基础标题序号排，否则会与已有本地标签重名。
        let (session_id, base_title, is_local) = self.workspace.tab(tab_id).map(|tab| {
            (
                tab.session_id,
                tab.session.name.clone(),
                matches!(tab.session.connection, Connection::Local(_)),
            )
        })?;
        if is_local {
            self.workspace
                .create_local_group(tab_id, session_id, &base_title);
        } else {
            self.workspace.create_group(tab_id, session_id);
        }
        self.workspace.reconcile_split_states();
        Some(tab_id)
    }

    /// 选中某个窗格：标签栏窗格列表与方向键共用一条规则。
    ///
    /// 放大态下“选中另一个窗格”是换看它，而不是退出放大，否则用户点一次列表
    /// 就把自己摆好的最大化视图丢掉了。
    pub(crate) fn select_pane(&mut self, tab_id: TabId) {
        if let Some(group_index) = self.workspace.group_index_for_tab(tab_id)
            && self.workspace.groups[group_index].zoomed.is_some()
        {
            self.workspace.groups[group_index].zoomed = Some(tab_id);
        }
    }

    /// 沿指定方向寻找相邻窗格；该方向已到边时交给并排格继续处理。
    pub(crate) fn focus_neighbor_pane(&self, direction: PaneDirection) -> SplitFocusPlan {
        let Some(group_index) = self.active_group_index() else {
            return SplitFocusPlan::NeighborTiling(direction);
        };
        let active_pane = self.workspace.groups[group_index].active_pane;
        match self.workspace.groups[group_index]
            .root
            .focus_neighbor(active_pane, direction)
        {
            Some(next) => SplitFocusPlan::Pane(next),
            None => SplitFocusPlan::NeighborTiling(direction),
        }
    }

    fn active_group_index(&self) -> Option<usize> {
        let tab_id = self.workspace.active_tab?;
        self.workspace.group_index_for_tab(tab_id)
    }

    fn session_for_tab(&self, tab_id: TabId) -> Option<Session> {
        let tab = self.workspace.tab(tab_id)?;
        Some(
            self.catalog
                .sessions
                .iter()
                .find(|session| session.id == tab.session_id)
                .cloned()
                .unwrap_or_else(|| tab.session.clone()),
        )
    }
}
