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
            .and_then(|tab| tab.connection_id());
        let has_other_remote = self.state.workspace.tabs.iter().any(|tab| {
            tab.id != tab_id
                && tab.connection_id().is_some()
                && tab.connection_id() != current_conn
                && tab
                    .session()
                    .is_some_and(|session| !matches!(session.connection, Connection::Local(_)))
        });
        if !has_other_remote {
            if !view.read(cx).mount_candidates().is_empty() {
                view.update(cx, |view, _| view.set_mount_candidates(Vec::new()));
            }
            return;
        }
        let is_local_only = crate::sftp::sftp_state(&self.state.workspace, tab_id, cx)
            .is_some_and(|panel| panel.local.only);
        let current_session = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.session_id());

        // 先只比较连接标识：大多数帧候选不变，此时不必克隆显示名与 cwd。
        let mut seen = std::collections::HashSet::new();
        if !is_local_only && let Some(session_id) = current_session {
            seen.insert(session_id);
        }
        let mut candidate_indices = Vec::new();
        for (index, tab) in self.state.workspace.tabs.iter().enumerate() {
            if tab.id == tab_id {
                continue;
            }
            let Some(connection_id) = tab.connection_id() else {
                continue;
            };
            if Some(connection_id) == current_conn {
                continue;
            }
            let Some(session) = tab.session() else {
                continue;
            };
            if matches!(session.connection, Connection::Local(_)) {
                continue;
            }
            if !seen.insert(session.id) {
                continue;
            }
            candidate_indices.push(index);
        }
        let unchanged = {
            let view = view.read(cx);
            let current = view.mount_candidates();
            !candidate_indices.is_empty()
                && current.len() == candidate_indices.len()
                && current.iter().zip(&candidate_indices).all(|(old, index)| {
                    self.state.workspace.tabs[*index].connection_id() == Some(old.connection_id)
                })
        };
        if unchanged {
            return;
        }

        let mut candidates = Vec::with_capacity(candidate_indices.len());
        for index in candidate_indices {
            let tab = &self.state.workspace.tabs[index];
            let Some(connection_id) = tab.connection_id() else {
                continue;
            };
            candidates.push(crate::sftp::SftpMountCandidate {
                connection_id,
                display_name: tab.display_name(),
                view: tab.sftp_view(),
                cwd: tab.cwd().map(str::to_string),
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
                crate::editor::open_editor_goto_line(&mut self.state, editor, window, cx);
            }
            EditorPanelEvent::FocusSftpPanel => {
                crate::sftp::focus_active_sftp_panel(&self.state, window, cx)
            }
            EditorPanelEvent::ResumePendingClose => {
                crate::editor::defer_editor_close_resume(&mut self.state, window, cx);
            }
            EditorPanelEvent::OpenLocalDiff(source) => {
                crate::diff_viewer::DiffViewerOps::new(&mut self.state)
                    .open_editor_local_diff(source, cx);
            }
            EditorPanelEvent::ListOpenChanged => cx.notify(),
            EditorPanelEvent::DiffPreferencesChanged => {
                let snapshot = crate::app::app_settings(cx).clone();
                self.state.persist_settings_reported(&snapshot);
            }
        }
    }

    pub(super) fn refresh_monitor_stats(&mut self, cx: &mut Context<Self>) {
        let stats = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id))
            .and_then(|tab| tab.system_stats());
        // 先借用比较：快照未变化时避免每帧克隆整份统计。
        if self.state.features.monitor.read(cx).stats() == stats {
            return;
        }
        let stats = stats.cloned();
        self.state
            .features
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
                crate::monitor::apply_monitor_drawer_width(&mut self.state, width, window, cx);
            }
            MonitorPanelEvent::MaximizeToTab => {
                crate::monitor::maximize_monitor_to_tab(&mut self.state, window, cx);
            }
            MonitorPanelEvent::DockToDrawer {
                tab_id,
                target_tab_id,
            } => {
                crate::monitor::dock_monitor_to_drawer(
                    &mut self.state,
                    tab_id,
                    target_tab_id,
                    window,
                    cx,
                );
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
            Some((tab.session_id()?, pane.forward_statuses.clone()))
        }) else {
            return;
        };
        // 先借用会话里的规则做比较：未变化时不必每帧克隆整份转发列表。
        let rules = self
            .state
            .session
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
                crate::editor::open_remote_editor_panel(&mut self.state, params, window, cx);
            }
            SftpPanelEvent::OpenInputModal(params) => {
                self.state.ui.modals.sftp.open_input(params, window, cx);
            }
            SftpPanelEvent::GrowPanelWidth(extra) => {
                crate::sftp::grow_sftp_panel_width(&mut self.state, extra, window, cx);
            }
            SftpPanelEvent::ChangeTerminalDirectory {
                connection_id,
                path,
            } => {
                self.change_terminal_directory(connection_id, path, window, cx);
            }
            _ => {}
        }
    }

    pub(super) fn flush_window_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::app::WindowRequest;
        if self.state.ui.window_requests.is_empty() {
            return;
        }
        let requests = self.state.ui.window_requests.take();
        for request in requests {
            match request {
                WindowRequest::Sftp(event) => self.handle_sftp_request(event, window, cx),
                WindowRequest::Monitor(event) => self.handle_monitor_request(event, window, cx),
                WindowRequest::Editor(event) => self.handle_editor_request(event, window, cx),
                WindowRequest::Settings(event) => {
                    crate::settings::handle_settings_request(&mut self.state, event, window, cx)
                }
                WindowRequest::Container { tab_id, event } => {
                    self.handle_container_request(tab_id, &event, window, cx);
                }
                WindowRequest::ContainerSidebar(event) => {
                    crate::container::on_container_sidebar_event(
                        &mut self.state,
                        &event,
                        window,
                        cx,
                    );
                }
                WindowRequest::GitSidebar(event) => {
                    crate::git::actions::on_git_sidebar_event(&mut self.state, &event, window, cx);
                }
                WindowRequest::SessionSidebar(event) => {
                    crate::sidebar::on_session_sidebar_event(&mut self.state, &event, window, cx);
                }
                WindowRequest::ActivateTab(tab_id) => self.activate_tab(tab_id, window, cx),
                WindowRequest::OpenSettings(page) => self.open_settings_modal(page, window, cx),
                WindowRequest::Toast { message, success } => {
                    self.show_toast(message, success, window, cx)
                }
                WindowRequest::CloseTab(tab_id) => self.close_tab(tab_id, window, cx),
                WindowRequest::CloseGroup(group_id) => self.close_group(group_id, window, cx),
                WindowRequest::ClosePane(tab_id) => self.close_pane(tab_id, window, cx),
                WindowRequest::OpenSessionForm(session_id) => match session_id {
                    Some(session_id) => self.open_edit_session(session_id, window, cx),
                    None => self.open_new_session(window, cx),
                },
                WindowRequest::TransientStatus(message) => {
                    self.state.set_transient_status(message, cx)
                }
                WindowRequest::LocalTerminal(cwd) => {
                    crate::connection::spawn_local_terminal(&mut self.state, cwd, cx)
                }
                WindowRequest::EditorClose(request) => match request {
                    crate::editor::EditorCloseRequest::Group(group_id) => {
                        self.close_group(group_id, window, cx)
                    }
                    crate::editor::EditorCloseRequest::Pane(tab_id) => {
                        self.close_pane(tab_id, window, cx)
                    }
                    crate::editor::EditorCloseRequest::Session(session_id) => {
                        self.delete_session_by_id(session_id, cx)
                    }
                    crate::editor::EditorCloseRequest::Tiling(groups) => {
                        crate::tiling::close_tiling_groups(&mut self.state, groups, window, cx)
                    }
                },
                WindowRequest::Git(event) => match &event {
                    crate::git::GitWorkspaceEvent::OpenSettings => {
                        crate::git::actions::GitActions::new(&mut self.state)
                            .open_git_settings(window, cx);
                    }
                    crate::git::GitWorkspaceEvent::OpenNameModal {
                        project,
                        purpose,
                        initial,
                    } => {
                        crate::git::actions::modal_bridges::open_git_name_modal(
                            &mut self.state,
                            project,
                            purpose.clone(),
                            initial.clone(),
                            window,
                            cx,
                        );
                    }
                    crate::git::GitWorkspaceEvent::OpenWorktreeModal { project, start } => {
                        crate::git::actions::modal_bridges::open_git_worktree_modal(
                            &mut self.state,
                            project.clone(),
                            start,
                            window,
                            cx,
                        );
                    }
                    _ => {}
                },
            }
        }
    }

    pub(crate) fn forward_view_element(&self, _cx: &mut Context<Self>) -> AnyElement {
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

    /// 安排一次绘制后的副作用处理。
    ///
    /// 渲染期只登记请求，真正的子实体刷新、窗口请求排空、弹窗对账和错误通知
    /// 投递都在 `defer_in` 回调中执行。这样 `Render::render` 不再直接修改状态，
    /// 也避免在渲染借用期间打开弹窗或更新其他实体。
    pub(super) fn schedule_frame_effects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.ui.feedback.frame_effects_scheduled {
            return;
        }
        self.state.ui.feedback.frame_effects_scheduled = true;
        cx.defer_in(window, |this, window, cx| {
            this.state.ui.feedback.frame_effects_scheduled = false;
            this.run_frame_effects(window, cx);
            crate::dialog_host::reconcile_dialogs(&mut this.state, window, cx);
            let pending_errors = this.state.ui.feedback.pending_notices.drain();
            for message in pending_errors {
                window.push_notification(Notification::error(message).autohide(false), cx);
            }
        });
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
        crate::status_bar::refresh_status_metrics(&mut self.state, cx);
        self.flush_window_requests(window, cx);
    }
}

pub(super) fn sync_stats_sampling(state: &mut crate::app::AppState, cx: &mut Context<AppShell>) {
    let demand = state.features.monitor.read(cx).is_open()
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
    if demand == state.features.stats_sampling {
        return;
    }
    state.features.stats_sampling = demand;
    state.send_command(transom_core::HostCommand::SetStatsSampling { enabled: demand });
}
