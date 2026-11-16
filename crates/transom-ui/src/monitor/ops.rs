//! 监控看板的采样生命周期、轮询调度与进程信号发送。
//!
//! 设计原理（Why）：采样调度是监控域能力，不应通过自由函数接收整个应用外壳。
//! [`MonitorOps`] 只借用它实际需要的工作区、任务、命令通道与反馈状态，既保留
//! 跨标签目标解析和后台任务生命周期，又让本模块可以脱离 `AppShell` 做纯逻辑测试。

use super::types::SampleIntervalMode;
use super::{MonitorPanelEvent, MonitorSectionCollapse, MonitorView};
use crate::app::{
    AppShell, AppState, PendingNotices, Persistence, TaskHub, WindowRequests, app_settings,
    command_send_error_text, update_app_settings,
};
use crate::workspace::WorkspaceState;
use gpui_kit::{Context, Entity};
use transom_core::{AppCommand, AppError, ConnectionId, HostCommand, ProcessInfo, TabId};

/// 打开监控抽屉后需要外壳补齐的终端目标。
pub(crate) enum MonitorTerminalPlan {
    /// 已有本地终端标签，只需激活它。
    Activate(TabId),
    /// 没有可复用的本地终端，需要新建。
    OpenLocal,
}

/// 监控域的窄操作上下文。
///
/// 字段只覆盖采样、任务和窗口请求所必需的状态；不持有 `AppShell`，也不暴露
/// 整个 `AppState`。调用方在应用边界从 `AppState` 拆借这些字段后传入。
pub(crate) struct MonitorOps<'a> {
    monitor: &'a Entity<MonitorView>,
    workspace: &'a mut WorkspaceState,
    tasks: &'a mut TaskHub,
    command_tx: &'a flume::Sender<AppCommand>,
    pending_notices: &'a mut PendingNotices,
    window_requests: &'a mut WindowRequests,
    persistence: &'a Persistence,
    status: &'a mut Option<String>,
}

impl<'a> MonitorOps<'a> {
    /// 从应用状态拆借监控域能力。
    pub(crate) fn new(state: &'a mut AppState) -> Self {
        Self {
            monitor: &state.features.monitor,
            workspace: &mut state.workspace,
            tasks: &mut state.runtime.tasks,
            command_tx: &state.runtime.services.command_tx,
            pending_notices: &mut state.ui.feedback.pending_notices,
            window_requests: &mut state.ui.window_requests,
            persistence: &state.runtime.persistence,
            status: &mut state.ui.status,
        }
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

    /// 处理监控面板发来的意图。
    ///
    /// 只用事件数据，不回读面板：事件派发阶段面板仍被租借。需要窗口的动作
    /// （脱离为标签页、收回抽屉、调整宽度）排到渲染期执行。
    pub(crate) fn on_monitor_panel_event(
        &mut self,
        event: &MonitorPanelEvent,
        cx: &mut Context<AppShell>,
    ) {
        match event {
            MonitorPanelEvent::CollapseChanged {
                collapse,
                expanded_processes,
            } => self.after_monitor_collapse(*collapse, *expanded_processes, cx),
            MonitorPanelEvent::RestartSampling(interval) => {
                self.restart_monitor_task(*interval, cx)
            }
            MonitorPanelEvent::SampleProcesses { collapsed } => {
                self.sample_processes(*collapsed, cx)
            }
            MonitorPanelEvent::TraceProcess(pid) => self.trace_process_in_terminal(*pid, cx),
            MonitorPanelEvent::KillProcess { pid, force } => {
                self.send_kill_process(*pid, *force, cx)
            }
            MonitorPanelEvent::RestartService(unit) => self.restart_systemd_service(unit, cx),
            MonitorPanelEvent::DrawerWidthChanged(_)
            | MonitorPanelEvent::MaximizeToTab
            | MonitorPanelEvent::DockToDrawer { .. }
            | MonitorPanelEvent::ClosePane { .. } => {
                self.window_requests.push_monitor(event.clone());
            }
        }
    }

    /// 活动栏入口：打开/关闭监控抽屉，打开时立即拉取一轮进程快照。
    pub(crate) fn toggle_monitor(
        &mut self,
        cx: &mut Context<AppShell>,
    ) -> Option<MonitorTerminalPlan> {
        let open = !self.monitor.read(cx).is_open();
        self.monitor.update(cx, |view, _| {
            view.set_open(open);
            if open {
                view.reset_history();
            }
        });
        let mut plan = None;
        if open {
            plan = self.ensure_terminal_for_monitor();
            let collapsed = self.monitor.read(cx).processes_collapsed();
            self.sample_processes(collapsed, cx);
            let interval = self.monitor.read(cx).interval_mode().interval();
            self.ensure_monitor_task(interval, cx);
        }
        cx.notify();
        plan
    }

    /// 监控抽屉打开时确保有一个可采样的终端目标。
    fn ensure_terminal_for_monitor(&mut self) -> Option<MonitorTerminalPlan> {
        let has_focused_terminal = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
            .is_some_and(|tab| tab.session_pane().is_some());
        if has_focused_terminal {
            return None;
        }
        let existing_local = self.workspace.tabs.iter().find_map(|tab| {
            (matches!(tab.session.connection, transom_core::Connection::Local(_))
                && tab.session_pane().is_some())
            .then_some(tab.id)
        });
        Some(existing_local.map_or(
            MonitorTerminalPlan::OpenLocal,
            MonitorTerminalPlan::Activate,
        ))
    }

    /// 保证进程轮询任务已启动；任务常驻但在面板关闭或进程卡片收起时不发命令。
    pub(crate) fn ensure_monitor_task(
        &mut self,
        interval: std::time::Duration,
        cx: &mut Context<AppShell>,
    ) {
        if self.tasks.background.monitor.is_some() {
            return;
        }
        self.tasks.background.monitor = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let result = this.update(cx, |app_state, cx| {
                    MonitorOps::new(&mut app_state.state).sample_processes_if_open(cx)
                });
                if result.is_err() {
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
        cx: &mut Context<AppShell>,
    ) {
        self.tasks.background.monitor = None;
        self.ensure_monitor_task(interval, cx);
        cx.notify();
    }

    /// 切换采样频率档位（1s/3s/10s），并重启轮询任务。
    pub(crate) fn set_monitor_interval_mode(
        &mut self,
        mode: SampleIntervalMode,
        cx: &mut Context<AppShell>,
    ) {
        if self
            .monitor
            .update(cx, |view, _| view.set_interval_mode(mode))
        {
            self.tasks.background.monitor = None;
            let interval = self.monitor.read(cx).interval_mode().interval();
            self.ensure_monitor_task(interval, cx);
            cx.notify();
        }
    }

    fn sample_processes_if_open(&mut self, cx: &mut Context<AppShell>) {
        if !self.monitor.read(cx).is_open() {
            return;
        }
        let collapsed = self.monitor.read(cx).processes_collapsed();
        self.sample_processes(collapsed, cx);
    }

    /// 采样进程列表：卡片折叠时只取总数维持摘要计数，展开时拉全量明细。
    ///
    /// `collapsed` 由调用方给出而不是在这里读面板实体：面板自身的点击回调执行
    /// 期间实体正被租借，此时再读会触发 GPUI 的双重租借 panic。
    pub(crate) fn sample_processes(&mut self, collapsed: bool, cx: &mut Context<AppShell>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        self.send_command(if collapsed {
            HostCommand::SampleProcessCount {
                tab_id,
                connection_id,
            }
        } else {
            HostCommand::SampleProcesses {
                tab_id,
                connection_id,
            }
        });
        cx.notify();
    }

    /// 折叠状态变更后的收尾：落盘展开开关，必要时补一轮采样。
    ///
    /// 状态切换由面板自己完成（见 `MonitorView::toggle_collapse`）。这里刻意
    /// 不读写面板实体：面板的点击回调执行期间实体正被租借，再访问会 panic。
    pub(crate) fn after_monitor_collapse(
        &mut self,
        collapse: MonitorSectionCollapse,
        expanded_processes: bool,
        cx: &mut Context<AppShell>,
    ) {
        update_app_settings(cx, |settings| {
            settings.monitor.cpu_expanded = !collapse.cpu;
            settings.monitor.memory_expanded = !collapse.memory;
            settings.monitor.disk_expanded = !collapse.disk;
            settings.monitor.network_expanded = !collapse.network;
            settings.monitor.ports_expanded = !collapse.ports;
            settings.monitor.services_expanded = !collapse.services;
            settings.monitor.processes_expanded = !collapse.processes;
        });
        let settings_snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&settings_snapshot);
        if expanded_processes {
            self.sample_processes(false, cx);
        }
        cx.notify();
    }

    /// 向当前活动终端发送一行命令输入。
    pub(crate) fn send_terminal_input(&mut self, command: &str, cx: &mut Context<AppShell>) {
        let terminal = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
            .and_then(|tab| tab.terminal());
        if let Some(terminal) = terminal {
            terminal.update(cx, |view, cx| view.send_text(command, cx));
        }
    }

    /// 在当前终端中追踪指定进程（自动输入 htop 或 top）。
    pub(crate) fn trace_process_in_terminal(&mut self, pid: u32, cx: &mut Context<AppShell>) {
        let command = format!("htop -p {pid} 2>/dev/null || top -p {pid}\r");
        self.send_terminal_input(&command, cx);
    }

    /// 在终端中重启崩溃的 Systemd 服务。
    pub(crate) fn restart_systemd_service(&mut self, unit: &str, cx: &mut Context<AppShell>) {
        let command = format!("systemctl restart {unit}\r");
        self.send_terminal_input(&command, cx);
    }

    /// 安全终止进程（对核心系统进程施加保护拦截）。
    pub(crate) fn send_kill_process(&mut self, pid: u32, force: bool, cx: &mut Context<AppShell>) {
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

    fn apply_processes(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
        cx: &mut Context<AppShell>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor
            .update(cx, |view, _| view.set_processes(processes));
        cx.notify();
    }

    fn apply_process_count(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
        cx: &mut Context<AppShell>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor
            .update(cx, |view, _| view.set_process_total(count));
        cx.notify();
    }

    fn apply_failure(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<AppShell>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor.update(cx, |view, _| {
            view.set_error(Some(crate::palette::localized_error(&error)));
        });
        cx.notify();
    }

    fn apply_killed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
        cx: &mut Context<AppShell>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        match error {
            Some(error) => self.apply_failure(tab_id, connection_id, error, cx),
            None => {
                self.monitor.update(cx, |view, _| view.set_error(None));
                *self.status = Some(rust_i18n::t!("monitor.kill_sent", pid = pid).to_string());
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
        cx: &mut Context<AppShell>,
    ) {
        self.apply_processes(tab_id, connection_id, processes, cx);
    }

    /// 折叠态的进程总数采样落地。
    pub(crate) fn monitor_count_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
        cx: &mut Context<AppShell>,
    ) {
        self.apply_process_count(tab_id, connection_id, count, cx);
    }

    pub(crate) fn monitor_failed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<AppShell>,
    ) {
        self.apply_failure(tab_id, connection_id, error, cx);
    }

    pub(crate) fn monitor_killed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
        cx: &mut Context<AppShell>,
    ) {
        self.apply_killed(tab_id, connection_id, pid, error, cx);
    }

    fn send_command(&mut self, command: impl Into<AppCommand>) {
        if let Err(error) = self.command_tx.try_send(command.into()) {
            self.pending_notices.push(command_send_error_text(error));
        }
    }

    fn persist_settings_reported(&mut self, settings: &transom_core::AppSettings) {
        if let Err(error) = self.persistence.save_settings(settings) {
            log::error!("保存监控展开状态失败，界面仍是新值但下次启动会回退，原因: {error}");
            self.pending_notices
                .push(crate::palette::localized_error(&error));
        }
    }
}
