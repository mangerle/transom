//! 监控看板的采样生命周期、轮询调度与进程信号发送。

use super::types::SampleIntervalMode;
use crate::app::{AppShell, TabContent, TerminalTab};
use gpui_kit::{Context, Window};
use transom_core::{
    AppError, ConnectionId, HostCommand, ProcessInfo, ProcessSort, SystemStats, TabId,
};

impl AppShell {
    /// 活动栏入口：打开/关闭监控抽屉，打开时立即拉取一轮进程快照。
    pub(crate) fn toggle_monitor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.monitor.open = !self.monitor.open;
        if self.monitor.open {
            self.monitor.history_tab = None;
            self.monitor.error = None;
            let has_focused_terminal = self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tab(tab_id))
                .is_some_and(|tab| tab.session_pane().is_some());
            if !has_focused_terminal {
                let existing_local = self
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
            self.sample_processes(cx);
            self.ensure_monitor_task(cx);
        } else {
            self.monitor.processes.clear();
            self.monitor.process_total = 0;
        }
        cx.notify();
    }

    /// 保证进程轮询任务已启动；任务常驻但在面板关闭或进程卡片收起时不发命令。
    pub(crate) fn ensure_monitor_task(&mut self, cx: &mut Context<Self>) {
        if self.tasks.monitor.is_some() {
            return;
        }
        let interval = self.monitor.interval_mode.interval();
        self.tasks.monitor = Some(cx.spawn(async move |this, cx| {
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

    /// 切换采样频率档位（1s/3s/10s），并重启轮询任务。
    pub(crate) fn set_monitor_interval_mode(
        &mut self,
        mode: SampleIntervalMode,
        cx: &mut Context<Self>,
    ) {
        if self.monitor.interval_mode != mode {
            self.monitor.interval_mode = mode;
            self.tasks.monitor = None;
            self.ensure_monitor_task(cx);
            cx.notify();
        }
    }

    fn sample_processes_if_open(&mut self, cx: &mut Context<Self>) {
        if !self.monitor.open {
            return;
        }
        self.sample_processes(cx);
    }

    /// 采样进程列表：卡片折叠时只取总数维持摘要计数，展开时拉全量明细。
    pub(crate) fn sample_processes(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        if self.monitor.collapse.processes {
            self.send_command(HostCommand::SampleProcessCount {
                tab_id,
                connection_id,
            });
        } else {
            self.send_command(HostCommand::SampleProcesses {
                tab_id,
                connection_id,
            });
        }
        cx.notify();
    }

    /// 点击表头列切换排序。
    ///
    /// 换列默认从降序开始（排行榜直觉），再次点同一列则翻转方向。
    /// 排序只发生在本地，不重新采样，切换是即时的。
    pub(crate) fn toggle_process_sort(&mut self, sort: ProcessSort, cx: &mut Context<Self>) {
        if self.monitor.sort == sort {
            self.monitor.sort_ascending = !self.monitor.sort_ascending;
        } else {
            self.monitor.sort = sort;
            self.monitor.sort_ascending = false;
        }
        cx.notify();
    }

    /// 切换各板块展开/折叠状态。
    pub(crate) fn toggle_section_collapse(&mut self, section: &str, cx: &mut Context<Self>) {
        match section {
            "cpu" => self.monitor.collapse.cpu = !self.monitor.collapse.cpu,
            "memory" => self.monitor.collapse.memory = !self.monitor.collapse.memory,
            "disk" => self.monitor.collapse.disk = !self.monitor.collapse.disk,
            "network" => self.monitor.collapse.network = !self.monitor.collapse.network,
            "ports" => self.monitor.collapse.ports = !self.monitor.collapse.ports,
            "services" => self.monitor.collapse.services = !self.monitor.collapse.services,
            "processes" => {
                self.monitor.collapse.processes = !self.monitor.collapse.processes;
                if !self.monitor.collapse.processes {
                    self.sample_processes(cx);
                }
            }
            _ => {}
        }
        self.settings.monitor.cpu_expanded = !self.monitor.collapse.cpu;
        self.settings.monitor.memory_expanded = !self.monitor.collapse.memory;
        self.settings.monitor.disk_expanded = !self.monitor.collapse.disk;
        self.settings.monitor.network_expanded = !self.monitor.collapse.network;
        self.settings.monitor.ports_expanded = !self.monitor.collapse.ports;
        self.settings.monitor.services_expanded = !self.monitor.collapse.services;
        self.settings.monitor.processes_expanded = !self.monitor.collapse.processes;
        let _ = self.services.settings_store.save(&self.settings);
        cx.notify();
    }

    /// 向当前活动终端发送一行命令输入。
    pub(crate) fn send_terminal_input(&mut self, cmd: &str, cx: &mut Context<Self>) {
        if let Some(tab_id) = self.workspace.active_tab
            && let Some(tab) = self.workspace.tab(tab_id)
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

    /// 安全终止进程（对核心系统进程施加保护拦截）。
    pub(crate) fn send_kill_process_safe(&mut self, pid: u32, force: bool, cx: &mut Context<Self>) {
        if pid <= 1 {
            self.monitor.error = Some(rust_i18n::t!("monitor.error_kill_pid1").to_string());
            cx.notify();
            return;
        }
        self.send_kill_process(pid, force, cx);
    }

    pub(crate) fn send_kill_process(&mut self, pid: u32, force: bool, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        self.send_command(HostCommand::KillProcess {
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
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor.processes = processes;
        self.monitor.process_total =
            u32::try_from(self.monitor.processes.len()).unwrap_or(u32::MAX);
        self.monitor.error = None;
        cx.notify();
    }

    fn on_process_count_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
        cx: &mut Context<Self>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor.process_total = count;
        self.monitor.error = None;
        cx.notify();
    }

    fn on_processes_failed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<Self>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor.error = Some(crate::palette::localized_error(&error));
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
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        match error {
            Some(error) => {
                self.monitor.error = Some(crate::palette::localized_error(&error));
            }
            None => {
                self.monitor.error = None;
                self.status = Some(rust_i18n::t!("monitor.kill_sent", pid = pid).to_string());
                self.sample_processes(cx);
            }
        }
        cx.notify();
    }

    /// 记录完整指标时序快照。
    pub(crate) fn monitor_record_stats(&mut self, tab_id: TabId, stats: &SystemStats) {
        if self.monitor.open {
            self.monitor.record_stats(tab_id, stats);
        }
    }

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

    /// 快速切换监控抽屉宽度（紧凑 420px / 宽屏 600px），同步通知 ResizableState 重新分配尺寸并持久化。
    pub(crate) fn toggle_monitor_drawer_width(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_width = if self.monitor.drawer_width > 480.0 {
            420.0
        } else {
            600.0
        };
        self.monitor.drawer_width = new_width;
        self.settings.monitor.drawer_width = new_width;
        self.persist_layout_settings(cx);
        let panels_state = if self.settings.window.sftp_panel_side.is_left() {
            &self.workspace_panels_left
        } else {
            &self.workspace_panels
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
        let active_tab_id = self.workspace.active_tab;
        let target_tab_id = active_tab_id
            .and_then(|id| self.workspace.tab(id))
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
                self.workspace
                    .tabs
                    .iter()
                    .find(|tab| !tab.is_player() && !tab.is_monitor())
                    .map(|tab| tab.id)
            });

        let Some(target_tab_id) = target_tab_id else {
            return;
        };

        // 2. 如果已经存在针对该 target_tab_id 的监控 Tab，直接切过去
        if let Some(existing) = self.workspace.tabs.iter().find(|t| {
            if let TabContent::Monitor(tid) = t.content {
                tid == target_tab_id
            } else {
                false
            }
        }) {
            let existing_id = existing.id;
            self.monitor.open = false;
            self.activate_tab(existing_id, window, cx);
            cx.notify();
            return;
        }

        // 3. 构建全屏监控独立标签页
        let Some(target_tab) = self.workspace.tab(target_tab_id) else {
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

        self.create_group(monitor_tab_id, target_session_id);
        self.workspace.tabs.push(new_tab);
        self.monitor.open = false;
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
        // 1. 打开抽屉
        self.monitor.open = true;

        // 2. 切回原会话标签页
        if self.workspace.tab(target_tab_id).is_some() {
            self.activate_tab(target_tab_id, window, cx);
        }

        // 3. 关闭全屏监控 Tab
        self.close_pane(tab_id, window, cx);
        cx.notify();
    }
}
