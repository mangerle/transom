//! 外壳的帧副作用：窗口请求排空、面板数据刷新与采样节流。
//!
//! 这些方法只在渲染帧内被调用，统一放在这里便于审阅“渲染期允许做什么”；
//! 面板事件本身仍由各自的事件处理模块负责。

use super::*;

impl AppShell {
    pub(super) fn refresh_sftp_mount_candidates(&mut self, cx: &mut Context<Self>) {
        use transom_core::Connection;

        let Some(tab_id) = self.state.workspace.active_tab else {
            return;
        };
        let Some(view) = self.state.sftp_view(tab_id) else {
            return;
        };
        let current_conn = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id);
        let is_local_only = self
            .sftp_state(tab_id, cx)
            .is_some_and(|panel| panel.local.only);
        let current_session = self.state.workspace.tab(tab_id).map(|tab| tab.session.id);

        // 先只比较连接标识：大多数帧候选不变，此时不必克隆显示名与 cwd。
        let mut seen = std::collections::HashSet::new();
        if !is_local_only && let Some(session_id) = current_session {
            seen.insert(session_id);
        }
        let mut candidate_ids = Vec::new();
        for tab in &self.state.workspace.tabs {
            if tab.id == tab_id {
                continue;
            }
            let Some(connection_id) = tab.connection_id else {
                continue;
            };
            if Some(connection_id) == current_conn {
                continue;
            }
            if matches!(&tab.session.connection, Connection::Local(_)) {
                continue;
            }
            if !seen.insert(tab.session.id) {
                continue;
            }
            candidate_ids.push(connection_id);
        }
        let unchanged = {
            let view = view.read(cx);
            let current = view.mount_candidates();
            !candidate_ids.is_empty()
                && current.len() == candidate_ids.len()
                && current
                    .iter()
                    .zip(&candidate_ids)
                    .all(|(old, id)| old.connection_id == *id)
        };
        if unchanged {
            return;
        }

        let mut seen = std::collections::HashSet::new();
        if !is_local_only && let Some(session_id) = current_session {
            seen.insert(session_id);
        }
        let mut candidates = Vec::with_capacity(candidate_ids.len());
        for tab in &self.state.workspace.tabs {
            if tab.id == tab_id {
                continue;
            }
            let Some(connection_id) = tab.connection_id else {
                continue;
            };
            if Some(connection_id) == current_conn {
                continue;
            }
            if matches!(&tab.session.connection, Connection::Local(_)) {
                continue;
            }
            if !seen.insert(tab.session.id) {
                continue;
            }
            candidates.push(crate::sftp::SftpMountCandidate {
                connection_id,
                display_name: tab.display_name(),
                view: tab.sftp_view(),
                cwd: tab.cwd.clone(),
            });
        }
        view.update(cx, |view, _| view.set_mount_candidates(candidates));
    }

    pub(super) fn handle_editor_request(
        &mut self,
        request: crate::editor::EditorPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::editor::EditorPanelEvent;
        match request {
            EditorPanelEvent::OpenGotoLine(editor) => {
                self.open_editor_goto_line(editor, window, cx);
            }
            EditorPanelEvent::FocusSftpPanel => self.focus_active_sftp_panel(window, cx),
            EditorPanelEvent::ResumePendingClose => {
                self.defer_editor_close_resume(window, cx);
            }
            EditorPanelEvent::OpenLocalDiff(source) => {
                self.open_editor_local_diff(source, cx);
            }
            EditorPanelEvent::ListOpenChanged => cx.notify(),
        }
    }

    pub(super) fn refresh_monitor_stats(&mut self, cx: &mut Context<Self>) {
        let stats = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id))
            .and_then(|tab| tab.system_stats().cloned());
        if self.state.monitor.read(cx).stats() == stats.as_ref() {
            return;
        }
        self.state
            .monitor
            .update(cx, |view, _| view.set_stats(stats));
    }

    pub(super) fn handle_monitor_request(
        &mut self,
        request: crate::monitor::MonitorPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::monitor::MonitorPanelEvent;
        match request {
            MonitorPanelEvent::DrawerWidthChanged(width) => {
                self.apply_monitor_drawer_width(width, window, cx);
            }
            MonitorPanelEvent::MaximizeToTab => {
                self.maximize_monitor_to_tab(window, cx);
            }
            MonitorPanelEvent::DockToDrawer {
                tab_id,
                target_tab_id,
            } => {
                self.dock_monitor_to_drawer(tab_id, target_tab_id, window, cx);
            }
            MonitorPanelEvent::ClosePane { tab_id } => {
                self.close_pane(tab_id, window, cx);
            }
            _ => {}
        }
    }

    pub(super) fn refresh_forward_panel_data(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.workspace.active_tab else {
            return;
        };
        let Some(view) = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.forwards_view())
        else {
            return;
        };
        let Some((session_id, statuses)) = self.state.workspace.tab(tab_id).and_then(|tab| {
            let pane = tab.session_pane()?;
            Some((tab.session_id, pane.forward_statuses.clone()))
        }) else {
            return;
        };
        // 先借用会话里的规则做比较：未变化时不必每帧克隆整份转发列表。
        let rules = self
            .state
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.forwards.as_slice())
            .unwrap_or_default();
        let unchanged = {
            let current = view.read(cx);
            current.rules() == rules && current.statuses() == &statuses
        };
        if unchanged {
            return;
        }
        view.update(cx, |view, _| {
            view.set_session_data(rules.to_vec(), statuses)
        });
    }

    pub(super) fn handle_sftp_request(
        &mut self,
        request: crate::sftp::SftpPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::sftp::SftpPanelEvent;
        match request {
            SftpPanelEvent::OpenEditor(params) => {
                self.open_remote_editor_panel(params, window, cx);
            }
            SftpPanelEvent::OpenInputModal(params) => {
                self.open_sftp_input_modal(params, window, cx);
            }
            SftpPanelEvent::GrowPanelWidth(extra) => {
                self.grow_sftp_panel_width(extra, window, cx);
            }
            _ => {}
        }
    }

    pub(super) fn flush_window_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::app::WindowRequest;
        if self.state.window_requests.is_empty() {
            return;
        }
        let requests = self.state.window_requests.take();
        for request in requests {
            match request {
                WindowRequest::Sftp(event) => self.handle_sftp_request(event, window, cx),
                WindowRequest::Monitor(event) => self.handle_monitor_request(event, window, cx),
                WindowRequest::Editor(event) => self.handle_editor_request(event, window, cx),
                WindowRequest::Settings(event) => self.handle_settings_request(event, window, cx),
            }
        }
    }

    pub(super) fn forward_view_element(&self, _cx: &mut Context<Self>) -> AnyElement {
        match self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id))
            .and_then(|tab| tab.forwards_view())
        {
            Some(view) => view.into_any_element(),
            None => div().into_any_element(),
        }
    }

    pub(super) fn run_frame_effects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        sync_stats_sampling(&mut self.state, cx);
        if let Some(error) = self.state.take_persistence_error() {
            self.state.set_error(localized_error(&error));
        }
        // 面板请求的"需要窗口"的动作在渲染期统一执行：事件派发阶段没有 Window。

        self.refresh_sftp_mount_candidates(cx);
        self.refresh_forward_panel_data(cx);
        self.refresh_monitor_stats(cx);
        self.refresh_status_metrics(cx);
        self.flush_window_requests(window, cx);
    }
}

pub(super) fn sync_stats_sampling(state: &mut crate::app::AppState, cx: &mut Context<AppShell>) {
    let demand = state.monitor.read(cx).is_open()
        || state.workspace.tabs.iter().any(|tab| tab.is_monitor())
        || app_settings(cx).status_bar.visible_items().any(|id| {
            matches!(
                id,
                transom_core::StatusItemId::Network
                    | transom_core::StatusItemId::Cpu
                    | transom_core::StatusItemId::Memory
                    | transom_core::StatusItemId::Disk
                    | transom_core::StatusItemId::Os
                    | transom_core::StatusItemId::Uptime
            )
        });
    if demand == state.stats_sampling {
        return;
    }
    state.stats_sampling = demand;
    state.send_command(transom_core::HostCommand::SetStatsSampling { enabled: demand });
}
