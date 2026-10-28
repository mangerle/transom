//! 会话与标签页管理模块入口。

mod close_ops;
mod duplicate;
mod session_ops;

#[cfg(test)]
mod tests;

use crate::app::{AppShell, TabContent, TabGroup, TabStatus, TerminalTab};
use crate::pane_tree::LayoutNode;
use gpui_kit::*;
use transom_core::{Connection, GroupId, SavedTabGroup, Session, SessionId, TabId};

impl AppShell {
    /// 新建标签组并返回组 ID；调用方随后把首个窗格加入 `tabs`。
    pub(crate) fn create_group(&mut self, tab_id: TabId, session_id: SessionId) -> GroupId {
        let serial = self
            .workspace
            .groups
            .iter()
            .filter(|group| group.session_id == session_id)
            .map(|group| group.serial)
            .max()
            .map_or(0, |max| max + 1);
        let group = TabGroup::new(tab_id, session_id, serial);
        let id = group.id;
        self.workspace.groups.push(group);
        id
    }

    /// 新建本地终端标签组，序号按"本地终端的基础标题"分配。
    pub(crate) fn create_local_group(
        &mut self,
        tab_id: TabId,
        session_id: SessionId,
        base_title: &str,
    ) -> GroupId {
        let serial = self
            .workspace
            .groups
            .iter()
            .filter_map(|group| {
                let tab = self
                    .workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.id == group.active_pane)?;
                if !matches!(tab.session.connection, Connection::Local(_)) {
                    return None;
                }
                (tab.session.name == base_title).then_some(group.serial)
            })
            .max()
            .map_or(0, |max| max + 1);
        let id = self.create_group(tab_id, session_id);
        if let Some(group) = self
            .workspace
            .groups
            .iter_mut()
            .find(|group| group.id == id)
        {
            group.serial = serial;
        }
        id
    }

    /// 根据窗口设置恢复一个标签组及其分割树布局，返回实际创建的标签组。
    pub(crate) fn restore_tab_group(
        &mut self,
        saved: &SavedTabGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<GroupId> {
        let pane_count = LayoutNode::saved_pane_count(&saved.layout).unwrap_or(1);

        let groups_before = self.workspace.groups.len();
        if saved.session == "local" {
            self.open_local_terminal(window, cx);
        } else {
            let session = self
                .catalog
                .sessions
                .iter()
                .find(|session| session.id.to_string() == saved.session)
                .cloned()?;
            self.connect_session(session.id, window, cx);
        }
        if self.workspace.groups.len() <= groups_before {
            return None;
        }
        let group_index = self.workspace.groups.len() - 1;
        let group_id = self.workspace.groups[group_index].id;
        if pane_count < 2 {
            return Some(group_id);
        }
        let first = self.workspace.groups[group_index].active_pane;
        let Some(session) = self.session_for_tab(first) else {
            return Some(group_id);
        };
        if matches!(session.connection, Connection::Serial(_)) {
            return Some(group_id);
        }
        let extra: Vec<TabId> = (1..pane_count).map(|_| TabId::new()).collect();
        let mut ids = std::iter::once(first).chain(extra.iter().copied());
        let Some(root) = LayoutNode::from_saved(&saved.layout, &mut ids) else {
            return Some(group_id);
        };
        self.workspace.groups[group_index].root = root;
        for pane_id in extra {
            self.workspace
                .tabs
                .push(TerminalTab::pending(pane_id, session.clone()));
            self.start_connection(pane_id, session.clone(), cx);
        }
        cx.notify();
        Some(group_id)
    }

    /// 取窗格应当使用的会话配置：优先目录里的最新记录，退回标签上的快照。
    pub(crate) fn session_for_tab(&self, tab_id: TabId) -> Option<Session> {
        let index = self.workspace.tab_index(tab_id)?;
        let session_id = self.workspace.tabs[index].session_id;
        Some(
            self.catalog
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .cloned()
                .unwrap_or_else(|| self.workspace.tabs[index].session.clone()),
        )
    }

    /// 将已打开标签的会话快照同步为目录中的最新配置。
    pub(crate) fn refresh_tab_sessions(&mut self) {
        for tab in &mut self.workspace.tabs {
            if let Some(session) = self
                .catalog
                .sessions
                .iter()
                .find(|session| session.id == tab.session_id)
            {
                tab.session = self.catalog.resolved_session(session);
            }
        }
    }

    pub(crate) fn group_index_for_tab(&self, tab_id: TabId) -> Option<usize> {
        self.workspace.group_index_of(tab_id)
    }

    /// 计算当前标签被移除或替换后应激活的窗格。
    ///
    /// 优先级：显式替换目标 → 并排网格当前焦点组 → 移除前的分组焦点 →
    /// 第一个分组。关闭标签、删除会话与复制标签共用同一顺序，避免几处
    /// 各写一条 `or_else` 链后逐渐漂移。
    pub(crate) fn fallback_active_pane(
        &self,
        replacement: Option<TabId>,
        previous_tiled_focus: Option<GroupId>,
    ) -> Option<TabId> {
        replacement
            .or_else(|| {
                self.active_tiling()
                    .and_then(|tiled| tiled.focused_group())
                    .and_then(|group| self.workspace.group(group).map(|item| item.active_pane))
            })
            .or_else(|| {
                previous_tiled_focus
                    .and_then(|group| self.workspace.group(group).map(|item| item.active_pane))
            })
            .or_else(|| self.workspace.groups.first().map(|group| group.active_pane))
    }

    pub(crate) fn reconnect_tab(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.workspace.tab_index(tab_id) else {
            return;
        };
        let Some(session) = self.session_for_tab(tab_id) else {
            return;
        };
        let tab = &mut self.workspace.tabs[index];
        tab.content = TabContent::Session(Box::default());
        tab.connection_id = None;
        tab.status = TabStatus::Connecting;
        tab.title = session.name.clone();
        self.workspace.active_tab = Some(tab_id);
        if let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.workspace.groups[group_index].active_pane = tab_id;
        }
        self.present_active_group();
        self.start_connection(tab_id, session, cx);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.workspace.active_tab
            && previous != tab_id
        {
            self.dismiss_sftp_hover_preview(previous, cx);
        }
        let group_id = self
            .group_index_for_tab(tab_id)
            .map(|index| self.workspace.groups[index].id);
        if let Some(group_id) = group_id
            && let Some(tiled) = self
                .workspace
                .tiled_sets
                .iter_mut()
                .find(|tiled| tiled.contains(group_id))
            && let Some(slot) = tiled.slot_of(group_id)
        {
            tiled.focused = slot;
        }
        self.workspace.active_tab = Some(tab_id);
        if let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.workspace.groups[group_index].active_pane = tab_id;
        }
        if let Some(terminal) = self.workspace.tab(tab_id).and_then(|tab| tab.terminal()) {
            let focus_handle = terminal.read(cx).focus_handle().clone();
            window.focus(&focus_handle, cx);
        }
        self.sync_broadcast_targets();
        cx.notify();
    }
}
