//! 监控看板的采样生命周期、轮询调度与进程信号发送。

use super::types::SampleIntervalMode;
use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::app::{AppShell, AppState, TabContent, TerminalTab};
use gpui_kit::{Context, Window};
use transom_core::{AppError, ConnectionId, HostCommand, ProcessInfo, TabId};

impl AppState {
    fn active_monitor_target(&self) -> Option<(TabId, ConnectionId)> {
        let tab_id = self.workspace.active_tab?;
        let tab = self.workspace.tab(tab_id)?;
        if !matches!(
            tab.session.connection,
            transom_core::Connection::Ssh(_) | transom_core::Connection::Local(_)
        ) {
            return None;
        }
        Some((tab_id, tab.connection_id?))
    }
}

impl AppShell {
    /// 活动栏入口：打开/关闭监控抽屉，打开时立即拉取一轮进程快照。
    pub(crate) fn toggle_monitor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = !self.state.monitor.read(cx).is_open();
        self.state.monitor.update(cx, |view, _| {
            view.set_open(open);
            if open {
                view.reset_history();
            }
        });
        if open {
            let has_focused_terminal = self
                .state
                .workspace
                .active_tab
                .and_then(|tab_id| self.state.workspace.tab(tab_id))
                .is_some_and(|tab| tab.session_pane().is_some());
            if !has_focused_terminal {
                let existing_local = self
                    .state
                    .workspace
                    .tabs
                    .iter()
                    .find(|tab| {
                        matches!(tab.session.connection, transom_core::Connection::Local(_))
                            && tab.session_pane().is_some()
                    })
                    .map(|tab| tab.id);
                if let Some(tab_id) = existing_local {
                    self.activate_tab(tab_id, window, cx);
                } else {
                    self.open_local_terminal(window, cx);
                }
            }
            self.sample_processes(self.state.monitor.read(cx).processes_collapsed(), cx);
            self.ensure_monitor_task(self.state.monitor.read(cx).interval_mode().interval(), cx);
        }
        cx.notify();
    }

    /// 保证进程轮询任务已启动；任务常驻但在面板关闭或进程卡片收起时不发命令。
    pub(crate) fn ensure_monitor_task(
        &mut self,
        interval: std::time::Duration,
        cx: &mut Context<Self>,
    ) {
        if self.state.tasks.monitor.is_some() {
            return;
        }
        self.state.tasks.monitor = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                if this
                    .update(cx, |this, cx| this.sample_processes_if_open(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// 面板已切换采样档位后重启轮询任务。
    ///
    /// 档位状态由面板自己写（见 `MonitorView::set_interval_mode`）：面板点击回调
    /// 执行期间实体被租借，这里不能再读写它。
    pub(crate) fn restart_monitor_task(
        &mut self,
        interval: std::time::Duration,
        cx: &mut Context<Self>,
    ) {
        self.state.tasks.monitor = None;
        self.ensure_monitor_task(interval, cx);
        cx.notify();
    }

    /// 切换采样频率档位（1s/3s/10s），并重启轮询任务。
    ///
    /// 仅用于面板之外（设置页）的调用点；面板内点击走
    /// [`Self::restart_monitor_task`]。
    pub(crate) fn set_monitor_interval_mode(
        &mut self,
        mode: SampleIntervalMode,
        cx: &mut Context<Self>,
    ) {
        if self
            .state
            .monitor
            .update(cx, |view, _| view.set_interval_mode(mode))
        {
            self.state.tasks.monitor = None;
            self.ensure_monitor_task(self.state.monitor.read(cx).interval_mode().interval(), cx);
            cx.notify();
        }
    }

    fn sample_processes_if_open(&mut self, cx: &mut Context<Self>) {
        if !self.state.monitor.read(cx).is_open() {
            return;
        }
        self.sample_processes(self.state.monitor.read(cx).processes_collapsed(), cx);
    }

    /// 采样进程列表：卡片折叠时只取总数维持摘要计数，展开时拉全量明细。
    ///
    /// `collapsed` 由调用方给出而不是在这里读面板实体：面板自身的点击回调执行
    /// 期间实体正被租借，此时再读会触发 GPUI 的双重租借 panic。
    pub(crate) fn sample_processes(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.state.active_monitor_target() else {
            return;
        };
        if collapsed {
            self.state.send_command(HostCommand::SampleProcessCount {
                tab_id,
                connection_id,
            });
        } else {
            self.state.send_command(HostCommand::SampleProcesses {
                tab_id,
                connection_id,
            });
        }
        cx.notify();
    }

    /// 折叠状态变更后的外壳侧收尾：落盘展开开关，必要时补一轮采样。
    ///
    /// 状态切换由面板自己完成（见 `MonitorView::toggle_collapse`）。这里刻意
    /// 不读写面板实体：面板的点击回调执行期间实体正被租借，再访问会 panic。
    pub(crate) fn after_monitor_collapse(
        &mut self,
        collapse: crate::monitor::MonitorSectionCollapse,
        expanded_processes: bool,
        cx: &mut Context<Self>,
    ) {
        update_app_settings(cx, |s| s.monitor.cpu_expanded = !collapse.cpu);
        update_app_settings(cx, |s| s.monitor.memory_expanded = !collapse.memory);
        update_app_settings(cx, |s| s.monitor.disk_expanded = !collapse.disk);
        update_app_settings(cx, |s| s.monitor.network_expanded = !collapse.network);
        update_app_settings(cx, |s| s.monitor.ports_expanded = !collapse.ports);
        update_app_settings(cx, |s| s.monitor.services_expanded = !collapse.services);
        update_app_settings(cx, |s| s.monitor.processes_expanded = !collapse.processes);
        let settings_snapshot = app_settings(cx).clone();
        self.state.persist_settings_reported(&settings_snapshot);
        if expanded_processes {
            // 刚展开进程卡片，按“未折叠”采样全量明细；这里不能再读面板实体，
            // 面板的点击回调执行期间它正被租借。
            self.sample_processes(false, cx);
        }
        cx.notify();
    }

    /// 向当前活动终端发送一行命令输入。
    pub(crate) fn send_terminal_input(&mut self, cmd: &str, cx: &mut Context<Self>) {
        if let Some(tab_id) = self.state.workspace.active_tab
            && let Some(tab) = self.state.workspace.tab(tab_id)
            && let Some(terminal) = tab.terminal()
        {
            terminal.update(cx, |view, cx| {
                view.send_text(cmd, cx);
            });
        }
    }

    /// 在当前终端中追踪指定进程（自动输入 htop 或 top）。
    pub(crate) fn trace_process_in_terminal(&mut self, pid: u32, cx: &mut Context<Self>) {
        let cmd = format!("htop -p {pid} 2>/dev/null || top -p {pid}\r");
        self.send_terminal_input(&cmd, cx);
    }

    /// 在终端中重启崩溃的 Systemd 服务。
    pub(crate) fn restart_systemd_service(&mut self, unit: &str, cx: &mut Context<Self>) {
        let cmd = format!("systemctl restart {unit}\r");
        self.send_terminal_input(&cmd, cx);
    }

    /// 当前活动标签的进程采样目标；只有 SSH 与本地终端支持。
    /// 安全终止进程（对核心系统进程施加保护拦截）。
    pub(crate) fn send_kill_process(&mut self, pid: u32, force: bool, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.state.active_monitor_target() else {
            return;
        };
        self.state.send_command(HostCommand::KillProcess {
            tab_id,
            connection_id,
            pid,
            force,
        });
        cx.notify();
    }

    fn on_processes_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
        cx: &mut Context<Self>,
    ) {
        if self.state.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.state
            .monitor
            .update(cx, |view, _| view.set_processes(processes));
        cx.notify();
    }

    fn on_process_count_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
        cx: &mut Context<Self>,
    ) {
        if self.state.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.state
            .monitor
            .update(cx, |view, _| view.set_process_total(count));
        cx.notify();
    }

    fn on_processes_failed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<Self>,
    ) {
        if self.state.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.state.monitor.update(cx, |view, _| {
            view.set_error(Some(crate::palette::localized_error(&error)));
        });
        cx.notify();
    }

    fn on_process_killed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        if self.state.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        match error {
            Some(error) => {
                self.state.monitor.update(cx, |view, _| {
                    view.set_error(Some(crate::palette::localized_error(&error)));
                });
            }
            None => {
                self.state
                    .monitor
                    .update(cx, |view, _| view.set_error(None));
                self.state.status = Some(rust_i18n::t!("monitor.kill_sent", pid = pid).to_string());
                self.sample_processes(false, cx);
            }
        }
        cx.notify();
    }

    /// 记录完整指标时序快照。
    pub(crate) fn monitor_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
        cx: &mut Context<Self>,
    ) {
        self.on_processes_sampled(tab_id, connection_id, processes, cx);
    }

    /// 折叠态的进程总数采样落地。
    pub(crate) fn monitor_count_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
        cx: &mut Context<Self>,
    ) {
        self.on_process_count_sampled(tab_id, connection_id, count, cx);
    }

    pub(crate) fn monitor_failed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<Self>,
    ) {
        self.on_processes_failed(tab_id, connection_id, error, cx);
    }

    pub(crate) fn monitor_killed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        self.on_process_killed(tab_id, connection_id, pid, error, cx);
    }

    /// 面板已改好宽度后，外壳侧同步持久化并让分栏重新分配尺寸。
    ///
    /// 宽度状态由面板写入（见 `MonitorView::set_drawer_width`）；面板点击回调
    /// 执行期间实体被租借，这里只做设置落盘与 `ResizableState` 调整。
    pub(crate) fn apply_monitor_drawer_width(
        &mut self,
        new_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        update_app_settings(cx, |s| s.monitor.drawer_width = new_width);
        self.persist_layout_settings(cx);
        let panels_state = if app_settings(cx).window.sftp_panel_side.is_left() {
            &self.state.workspace_panels_left
        } else {
            &self.state.workspace_panels
        };
        let panel_count = panels_state.read(cx).sizes().len();
        if panel_count > 0 {
            let monitor_ix = panel_count - 1;
            panels_state.update(cx, |state, cx| {
                state.resize_panel(monitor_ix, gpui_kit::px(new_width), window, cx);
            });
        }
        cx.notify();
    }

    /// 将监控抽屉脱离为全屏独立的 Workspace Tab。
    pub(crate) fn maximize_monitor_to_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 1. 寻找当前关联的主机会话 TabId
        let active_tab_id = self.state.workspace.active_tab;
        let target_tab_id = active_tab_id
            .and_then(|id| self.state.workspace.tab(id))
            .and_then(|tab| {
                if let TabContent::Monitor(tid) = tab.content {
                    Some(tid)
                } else if !tab.is_player() {
                    Some(tab.id)
                } else {
                    None
                }
            })
            .or_else(|| {
                self.state
                    .workspace
                    .tabs
                    .iter()
                    .find(|tab| !tab.is_player() && !tab.is_monitor())
                    .map(|tab| tab.id)
            });

        let Some(target_tab_id) = target_tab_id else {
            return;
        };

        // 2. 如果已经存在针对该 target_tab_id 的监控 Tab，直接切过去
        if let Some(existing) = self.state.workspace.tabs.iter().find(|t| {
            if let TabContent::Monitor(tid) = t.content {
                tid == target_tab_id
            } else {
                false
            }
        }) {
            let existing_id = existing.id;
            self.activate_tab(existing_id, window, cx);
            cx.notify();
            return;
        }

        // 3. 构建全屏监控独立标签页
        let Some(target_tab) = self.state.workspace.tab(target_tab_id) else {
            return;
        };
        let target_title = target_tab.display_name();
        let target_session = target_tab.session.clone();
        let target_connection_id = target_tab.connection_id;
        let target_session_id = target_tab.session_id;

        let monitor_tab_id = TabId::new();
        let tab_title = rust_i18n::t!("monitor.view.tab_name", host = &target_title).to_string();

        let new_tab = TerminalTab::monitor_tab(
            monitor_tab_id,
            target_session,
            tab_title,
            target_tab_id,
            target_connection_id,
        );

        self.state.create_group(monitor_tab_id, target_session_id);
        self.state.workspace.tabs.push(new_tab);
        self.activate_tab(monitor_tab_id, window, cx);
        cx.notify();
    }

    /// 将全屏监控独立标签页收回并停靠为右侧抽屉。
    #[allow(dead_code)]
    pub(crate) fn dock_monitor_to_drawer(
        &mut self,
        tab_id: TabId,
        target_tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 抽屉的开合由面板在调用前写好；这里只做标签切换与关闭。
        // 2. 切回原会话标签页
        if self.state.workspace.tab(target_tab_id).is_some() {
            self.activate_tab(target_tab_id, window, cx);
        }

        // 3. 关闭全屏监控 Tab
        self.close_pane(tab_id, window, cx);
        cx.notify();
    }
}
