//! 会话与标签页管理模块入口。

mod close_ops;
mod duplicate;
mod session_ops;

#[cfg(test)]
mod tests;

use crate::app::{AppShell, AppState, TabContent, TabGroup, TabStatus, TerminalTab};
use crate::pane_tree::LayoutNode;
use gpui_kit::*;
use transom_core::{Connection, GroupId, SavedTabGroup, Session, SessionId, TabId};

impl AppState {
    /// 标签所属分组的索引；标签不属于任何分组时返回 `None`。
    ///
    /// 纯状态查询，不依赖 GPUI 上下文；归属于 [`AppState`]，外壳显式经
    /// `self.state.group_index_for_tab` 调用。
    pub(crate) fn group_index_for_tab(&self, tab_id: TabId) -> Option<usize> {
        self.workspace.group_index_of(tab_id)
    }
}

impl AppState {
    pub(crate) fn refresh_tab_sessions(&mut self) {
        for index in 0..self.workspace.tabs.len() {
            let session_id = self.workspace.tabs[index].session_id;
            // 先结束对 catalog 的只读借用，再写回标签：两个字段同属
            // `self.state`，同时读写会被当成对整体的借用冲突。
            let resolved = self
                .catalog
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .map(|session| self.catalog.resolved_session(session));
            if let Some(session) = resolved {
                self.workspace.tabs[index].session = session;
            }
        }
    }
}

impl AppState {
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
}

impl AppShell {
    /// 新建标签组并返回组 ID；调用方随后把首个窗格加入 `tabs`。
    /// 根据窗口设置恢复一个标签组及其分割树布局，返回实际创建的标签组。
    pub(crate) fn restore_tab_group(
        &mut self,
        saved: &SavedTabGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<GroupId> {
        let pane_count = LayoutNode::saved_pane_count(&saved.layout).unwrap_or(1);

        let groups_before = self.state.workspace.groups.len();
        if saved.session == "local" {
            self.open_local_terminal(window, cx);
        } else {
            let session = self
                .state
                .catalog
                .sessions
                .iter()
                .find(|session| session.id.to_string() == saved.session)
                .cloned()?;
            self.connect_session(session.id, window, cx);
        }
        if self.state.workspace.groups.len() <= groups_before {
            return None;
        }
        let group_index = self.state.workspace.groups.len() - 1;
        let group_id = self.state.workspace.groups[group_index].id;
        if pane_count < 2 {
            return Some(group_id);
        }
        let first = self.state.workspace.groups[group_index].active_pane;
        let Some(session) = self.state.session_for_tab(first) else {
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
        self.state.workspace.groups[group_index].root = root;
        for pane_id in extra {
            self.state
                .workspace
                .tabs
                .push(TerminalTab::pending(pane_id, session.clone()));
            self.start_connection(pane_id, session.clone(), cx);
        }
        cx.notify();
        Some(group_id)
    }

    /// 取窗格应当使用的会话配置：优先目录里的最新记录，退回标签上的快照。
    /// 计算当前标签被移除或替换后应激活的窗格。
    ///
    /// 优先级：显式替换目标 → 并排网格当前焦点组 → 移除前的分组焦点 →
    /// 第一个分组。关闭标签、删除会话与复制标签共用同一顺序，避免几处
    /// 各写一条 `or_else` 链后逐渐漂移。
    pub(crate) fn reconnect_tab(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return;
        };
        let Some(session) = self.state.session_for_tab(tab_id) else {
            return;
        };
        let tab = &mut self.state.workspace.tabs[index];
        tab.content = TabContent::Session(Box::default());
        tab.connection_id = None;
        tab.status = TabStatus::Connecting;
        // 新连接尚未收到 OSC 0/2 与 OSC 7：标签回落会话名，SFTP 重新等待上报。
        tab.title.clear();
        tab.cwd = None;
        self.state.workspace.active_tab = Some(tab_id);
        if let Some(group_index) = self.state.group_index_for_tab(tab_id) {
            self.state.workspace.groups[group_index].active_pane = tab_id;
        }
        self.state.present_active_group();
        self.start_connection(tab_id, session, cx);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.state.workspace.active_tab
            && previous != tab_id
        {
            self.dismiss_sftp_hover_preview(previous, cx);
        }
        let group_id = self
            .state
            .group_index_for_tab(tab_id)
            .map(|index| self.state.workspace.groups[index].id);
        if let Some(group_id) = group_id
            && let Some(tiled) = self
                .state
                .workspace
                .tiled_sets
                .iter_mut()
                .find(|tiled| tiled.contains(group_id))
            && let Some(slot) = tiled.slot_of(group_id)
        {
            tiled.focused = slot;
        }
        self.state.workspace.active_tab = Some(tab_id);
        if let Some(group_index) = self.state.group_index_for_tab(tab_id) {
            self.state.workspace.groups[group_index].active_pane = tab_id;
        }
        if let Some(terminal) = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.terminal())
        {
            let focus_handle = terminal.read(cx).focus_handle().clone();
            window.focus(&focus_handle, cx);
        }
        self.state.sync_broadcast_targets();
        // 监控面板的重置排到绘制之后：本方法也会被监控面板自己的按钮调用
        // （脱离为标签页、收回抽屉），此刻面板实体正被租借，立即读写会双重租借。
        let app = cx.entity().clone();
        window.defer(cx, move |_window, cx| {
            app.update(cx, |shell, cx| {
                if shell.state.monitor.read(cx).is_open() {
                    shell
                        .state
                        .monitor
                        .update(cx, |view, _| view.reset_history());
                    let collapsed = shell.state.monitor.read(cx).processes_collapsed();
                    shell.sample_processes(collapsed, cx);
                }
            });
        });
        cx.notify();
    }
}
