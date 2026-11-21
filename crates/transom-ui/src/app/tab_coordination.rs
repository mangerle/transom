//! 跨面板/跨窗口的标签生命周期协调：打开恢复、重连、激活聚焦与关闭清理。
//!
//! 设计原理（Why）：
//! - 纯标签集合与分割树状态操作下沉在 `workspace/tabs/`（仅操作 `WorkspaceState` / `AppState`）；
//! - 涉及控制面命令发送、编辑器脏状态拦截、SFTP 预览收起、监控面板同步以及
//!   窗口关闭的协调动作集中留在 `app/`，彻底消除 `workspace/tabs/` 对 `AppShell` 的反向耦合。

use super::{AppShell, SessionTab, TabContent, TabStatus, WorkspaceTab, app_settings};
use crate::pane_tree::LayoutNode;
use crate::sftp::PendingEditorClose;
use crate::tiling::StripEntry;
use gpui_kit::*;
use transom_core::{
    Connection, ContainerCommand, GroupId, SavedTabGroup, Session, SessionCatalog, TabId,
    TerminalCommand,
};

impl AppShell {
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
            crate::connection::open_local_terminal(&mut self.state, window, cx);
        } else {
            let session = saved_session_for_key(&self.state.session.catalog, &saved.session)?;
            crate::connection::connect_session(&mut self.state, session.id, window, cx);
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
                .push(WorkspaceTab::new_session(pane_id, session.clone()));
            crate::connection::start_connection(&mut self.state, pane_id, session.clone(), cx);
        }
        cx.notify();
        Some(group_id)
    }

    /// 重新发起指定标签的连接。
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
        tab.content = TabContent::Session(Box::new(SessionTab {
            session_id: session.id,
            session: session.clone(),
            connection_id: None,
            cwd: None,
            progress: None,
            pane: Default::default(),
        }));
        tab.set_connection_id(None);
        tab.status = TabStatus::Connecting;
        // 新连接尚未收到 OSC 0/2 与 OSC 7：标签回落会话名，SFTP 重新等待上报。
        tab.title.clear();
        tab.set_cwd(None);
        self.state.workspace.active_tab = Some(tab_id);
        if let Some(group_index) = self.state.workspace.group_index_for_tab(tab_id) {
            self.state.workspace.groups[group_index].active_pane = tab_id;
        }
        self.state.present_active_group();
        crate::connection::start_connection(&mut self.state, tab_id, session, cx);
    }

    /// 激活指定标签并同步焦点、并排网格槽位及附属面板状态。
    pub(crate) fn activate_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.state.workspace.active_tab
            && previous != tab_id
            && let Some(view) = self.state.sftp_view(previous)
        {
            view.update(cx, |view, cx| view.dismiss_hover_preview(cx));
        }
        let group_id = self
            .state
            .workspace
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
        if let Some(group_index) = self.state.workspace.group_index_for_tab(tab_id) {
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
                if shell.state.features.monitor.read(cx).is_open() {
                    shell
                        .state
                        .features
                        .monitor
                        .update(cx, |view, _| view.reset_history());
                    let collapsed = shell.state.features.monitor.read(cx).processes_collapsed();
                    crate::monitor::MonitorOps::new(&mut shell.state)
                        .sample_processes(collapsed, cx);
                }
            });
        });
        cx.notify();
    }

    /// 关闭当前标签（或并排网格）之外的所有标签组。
    pub(crate) fn close_other_tabs(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_group) = self.state.workspace.group_index_for_tab(tab_id) else {
            return;
        };
        let target_group_id = self.state.workspace.groups[target_group].id;
        let mut keep = vec![target_group_id];
        if let Some(tiled) = self.state.tiling_of_group(target_group_id) {
            keep.extend(tiled.occupied_groups());
        }
        let others: Vec<GroupId> = self
            .state
            .workspace
            .groups
            .iter()
            .filter(|group| !keep.contains(&group.id))
            .map(|group| group.id)
            .collect();
        for group_id in others {
            self.close_group(group_id, window, cx);
        }
    }

    /// 关闭窗格所属的整个标签组。
    pub(crate) fn close_tab(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self
            .state
            .workspace
            .group_index_for_tab(tab_id)
            .map(|index| self.state.workspace.groups[index].id)
        else {
            return;
        };
        self.close_group(group_id, window, cx);
    }

    /// 关闭标签组，并回收其中全部窗格的连接。
    pub(crate) fn close_group(
        &mut self,
        group_id: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.state.workspace.group_index(group_id) else {
            return;
        };
        let panes = self.state.workspace.groups[index].pane_ids();
        if crate::editor::block_close_for_dirty_editors(
            &mut self.state,
            &panes,
            PendingEditorClose::Group(group_id),
            cx,
        ) {
            return;
        }
        let tiled_focus = self
            .state
            .tiling_of_group(group_id)
            .and_then(|tiled| tiled.focused_group());
        self.state.workspace.groups.remove(index);
        for tab_id in &panes {
            let action = self
                .state
                .workspace
                .tab(*tab_id)
                .map(WorkspaceTab::close_action);
            if let Some(action) = action {
                match action {
                    super::TabCloseAction::CloseTerminal { tab_id } => {
                        self.state
                            .send_command(TerminalCommand::CloseTab { tab_id });
                    }
                    super::TabCloseAction::DisconnectContainer { tab_id } => {
                        self.state
                            .send_command(ContainerCommand::Disconnect { tab_id });
                    }
                    super::TabCloseAction::DropOnly => {}
                }
            }
        }
        self.state
            .workspace
            .tabs
            .retain(|tab| !panes.contains(&tab.id));
        crate::editor::prune_tab_overlays(&mut self.state, cx);
        self.state.prune_tilings();

        let active_removed = self
            .state
            .workspace
            .active_tab
            .is_some_and(|id| panes.contains(&id));
        if active_removed {
            let tiled_return = self
                .state
                .active_tiling()
                .and_then(|tiled| tiled.focused_group())
                .or(tiled_focus)
                .and_then(|id| self.state.workspace.group(id))
                .map(|group| group.active_pane);
            self.state.workspace.active_tab = tiled_return.or_else(|| {
                self.state
                    .workspace
                    .groups
                    .get(index.saturating_sub(1))
                    .or_else(|| self.state.workspace.groups.first())
                    .map(|group| group.active_pane)
            });
            if let Some(active) = self.state.workspace.active_tab {
                self.activate_tab(active, window, cx);
            }
        }
        self.state.prune_tab_selection();
        self.state.sync_broadcast_targets();
        if self.state.workspace.groups.is_empty()
            && app_settings(cx).window.close_window_on_all_tabs_closed
        {
            crate::window_state::close_app(self, window, cx);
            return;
        }
        cx.notify();
    }

    /// 关闭单个窗格；组内最后一个窗格关闭时等价于关闭整个标签。
    pub(crate) fn close_pane(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group_index) = self.state.workspace.group_index_for_tab(tab_id) else {
            return;
        };
        if self.state.workspace.groups[group_index].pane_count() == 1 {
            let group_id = self.state.workspace.groups[group_index].id;
            self.close_group(group_id, window, cx);
            return;
        }
        if crate::editor::block_close_for_dirty_editors(
            &mut self.state,
            std::slice::from_ref(&tab_id),
            PendingEditorClose::Pane(tab_id),
            cx,
        ) {
            return;
        }

        self.state
            .send_command(TerminalCommand::CloseTab { tab_id });
        self.state.workspace.tabs.retain(|tab| tab.id != tab_id);
        crate::editor::prune_tab_overlays(&mut self.state, cx);
        let replacement = self.state.detach_pane_from_group(tab_id);
        self.state.prune_tilings();
        if let Some(next) = replacement {
            self.activate_tab(next, window, cx);
        }
        cx.notify();
    }

    /// 在标签栏条目之间轮转：并排网格算一条，组内成员不单独占位。
    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.state.strip_entries();
        if entries.len() <= 1 {
            return;
        }
        let current = self
            .state
            .active_group_id()
            .and_then(|group| {
                entries
                    .iter()
                    .position(|entry| self.state.entry_contains(*entry, group))
            })
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % entries.len()
        } else {
            (current + entries.len() - 1) % entries.len()
        };
        let target = match entries[next] {
            StripEntry::Single(group) => Some(group),
            StripEntry::Tiled { id, .. } => self
                .state
                .tiling_by_id(id)
                .and_then(|tiled| tiled.focused_group()),
        };
        if let Some(group) = target {
            crate::tiling::activate_group(&mut self.state, group, window, cx);
        }
    }
}

/// 按持久化会话键解析启动恢复目标。
///
/// 设计原理（Why）：旧设置中的未知键（包括历史 Git/容器占位 ID）必须安全跳过，
/// 不能阻断其他标签恢复；本地终端由调用方单独处理。
pub(crate) fn saved_session_for_key(catalog: &SessionCatalog, key: &str) -> Option<Session> {
    catalog
        .sessions
        .iter()
        .find(|session| session.id.to_string() == key)
        .cloned()
}
