//! 监控看板的采样生命周期、轮询调度与进程信号发送。

use super::types::SampleIntervalMode;
use crate::app::{AppShell, AppState, app_settings, update_app_settings};
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

/// 处理监控面板发来的意图。
///
/// 只用事件数据，不回读面板：事件派发阶段面板仍被租借。需要窗口的动作
/// （脱离为标签页、收回抽屉、调整宽度）排到渲染期执行。
pub(crate) fn on_monitor_panel_event(
    shell: &mut AppShell,
    event: &crate::monitor::MonitorPanelEvent,
    cx: &mut Context<AppShell>,
) {
    use crate::monitor::MonitorPanelEvent;
    match event {
        MonitorPanelEvent::CollapseChanged {
            collapse,
            expanded_processes,
        } => {
            crate::monitor::after_monitor_collapse(shell, *collapse, *expanded_processes, cx);
        }
        MonitorPanelEvent::RestartSampling(interval) => {
            crate::monitor::restart_monitor_task(shell, *interval, cx);
        }
        MonitorPanelEvent::SampleProcesses { collapsed } => {
            crate::monitor::sample_processes(shell, *collapsed, cx);
        }
        MonitorPanelEvent::TraceProcess(pid) => {
            crate::monitor::trace_process_in_terminal(shell, *pid, cx);
        }
        MonitorPanelEvent::KillProcess { pid, force } => {
            crate::monitor::send_kill_process(shell, *pid, *force, cx);
        }
        MonitorPanelEvent::RestartService(unit) => {
            crate::monitor::restart_systemd_service(shell, unit, cx);
        }
        MonitorPanelEvent::DrawerWidthChanged(_)
        | MonitorPanelEvent::MaximizeToTab
        | MonitorPanelEvent::DockToDrawer { .. }
        | MonitorPanelEvent::ClosePane { .. } => {
            shell.state.window_requests.push_monitor(event.clone());
        }
    }
}

/// 活动栏入口：打开/关闭监控抽屉，打开时立即拉取一轮进程快照。
pub(crate) fn toggle_monitor(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let open = !shell.state.monitor.read(cx).is_open();
    shell.state.monitor.update(cx, |view, _| {
        view.set_open(open);
        if open {
            view.reset_history();
        }
    });
    if open {
        let has_focused_terminal = shell
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| shell.state.workspace.tab(tab_id))
            .is_some_and(|tab| tab.session_pane().is_some());
        if !has_focused_terminal {
            let existing_local = shell
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
                shell.activate_tab(tab_id, window, cx);
            } else {
                crate::connection::open_local_terminal(shell, window, cx);
            }
        }
        crate::monitor::sample_processes(
            shell,
            shell.state.monitor.read(cx).processes_collapsed(),
            cx,
        );
        crate::monitor::ensure_monitor_task(
            shell,
            shell.state.monitor.read(cx).interval_mode().interval(),
            cx,
        );
    }
    cx.notify();
}

/// 保证进程轮询任务已启动；任务常驻但在面板关闭或进程卡片收起时不发命令。
pub(crate) fn ensure_monitor_task(
    shell: &mut AppShell,
    interval: std::time::Duration,
    cx: &mut Context<AppShell>,
) {
    if shell.state.tasks.background.monitor.is_some() {
        return;
    }
    shell.state.tasks.background.monitor = Some(cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(interval).await;
            if this.update(cx, sample_processes_if_open).is_err() {
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
    shell: &mut AppShell,
    interval: std::time::Duration,
    cx: &mut Context<AppShell>,
) {
    shell.state.tasks.background.monitor = None;
    crate::monitor::ensure_monitor_task(shell, interval, cx);
    cx.notify();
}

/// 切换采样频率档位（1s/3s/10s），并重启轮询任务。
///
/// 仅用于面板之外（设置页）的调用点；面板内点击走
/// [`crate::monitor::restart_monitor_task`]。
pub(crate) fn set_monitor_interval_mode(
    shell: &mut AppShell,
    mode: SampleIntervalMode,
    cx: &mut Context<AppShell>,
) {
    if shell
        .state
        .monitor
        .update(cx, |view, _| view.set_interval_mode(mode))
    {
        shell.state.tasks.background.monitor = None;
        crate::monitor::ensure_monitor_task(
            shell,
            shell.state.monitor.read(cx).interval_mode().interval(),
            cx,
        );
        cx.notify();
    }
}

fn sample_processes_if_open(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    if !shell.state.monitor.read(cx).is_open() {
        return;
    }
    crate::monitor::sample_processes(
        shell,
        shell.state.monitor.read(cx).processes_collapsed(),
        cx,
    );
}

/// 采样进程列表：卡片折叠时只取总数维持摘要计数，展开时拉全量明细。
///
/// `collapsed` 由调用方给出而不是在这里读面板实体：面板自身的点击回调执行
/// 期间实体正被租借，此时再读会触发 GPUI 的双重租借 panic。
pub(crate) fn sample_processes(shell: &mut AppShell, collapsed: bool, cx: &mut Context<AppShell>) {
    let Some((tab_id, connection_id)) = shell.state.active_monitor_target() else {
        return;
    };
    if collapsed {
        shell.state.send_command(HostCommand::SampleProcessCount {
            tab_id,
            connection_id,
        });
    } else {
        shell.state.send_command(HostCommand::SampleProcesses {
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
    shell: &mut AppShell,
    collapse: crate::monitor::MonitorSectionCollapse,
    expanded_processes: bool,
    cx: &mut Context<AppShell>,
) {
    update_app_settings(cx, |s| s.monitor.cpu_expanded = !collapse.cpu);
    update_app_settings(cx, |s| s.monitor.memory_expanded = !collapse.memory);
    update_app_settings(cx, |s| s.monitor.disk_expanded = !collapse.disk);
    update_app_settings(cx, |s| s.monitor.network_expanded = !collapse.network);
    update_app_settings(cx, |s| s.monitor.ports_expanded = !collapse.ports);
    update_app_settings(cx, |s| s.monitor.services_expanded = !collapse.services);
    update_app_settings(cx, |s| s.monitor.processes_expanded = !collapse.processes);
    let settings_snapshot = app_settings(cx).clone();
    shell.state.persist_settings_reported(&settings_snapshot);
    if expanded_processes {
        // 刚展开进程卡片，按“未折叠”采样全量明细；这里不能再读面板实体，
        // 面板的点击回调执行期间它正被租借。
        crate::monitor::sample_processes(shell, false, cx);
    }
    cx.notify();
}

/// 向当前活动终端发送一行命令输入。
pub(crate) fn send_terminal_input(shell: &mut AppShell, cmd: &str, cx: &mut Context<AppShell>) {
    if let Some(tab_id) = shell.state.workspace.active_tab
        && let Some(tab) = shell.state.workspace.tab(tab_id)
        && let Some(terminal) = tab.terminal()
    {
        terminal.update(cx, |view, cx| {
            view.send_text(cmd, cx);
        });
    }
}

/// 在当前终端中追踪指定进程（自动输入 htop 或 top）。
pub(crate) fn trace_process_in_terminal(
    shell: &mut AppShell,
    pid: u32,
    cx: &mut Context<AppShell>,
) {
    let cmd = format!("htop -p {pid} 2>/dev/null || top -p {pid}\r");
    crate::monitor::send_terminal_input(shell, &cmd, cx);
}

/// 在终端中重启崩溃的 Systemd 服务。
pub(crate) fn restart_systemd_service(
    shell: &mut AppShell,
    unit: &str,
    cx: &mut Context<AppShell>,
) {
    let cmd = format!("systemctl restart {unit}\r");
    crate::monitor::send_terminal_input(shell, &cmd, cx);
}

/// 当前活动标签的进程采样目标；只有 SSH 与本地终端支持。
/// 安全终止进程（对核心系统进程施加保护拦截）。
pub(crate) fn send_kill_process(
    shell: &mut AppShell,
    pid: u32,
    force: bool,
    cx: &mut Context<AppShell>,
) {
    let Some((tab_id, connection_id)) = shell.state.active_monitor_target() else {
        return;
    };
    shell.state.send_command(HostCommand::KillProcess {
        tab_id,
        connection_id,
        pid,
        force,
    });
    cx.notify();
}

fn on_processes_sampled(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    processes: Vec<ProcessInfo>,
    cx: &mut Context<AppShell>,
) {
    if shell.state.active_monitor_target() != Some((tab_id, connection_id)) {
        return;
    }
    shell
        .state
        .monitor
        .update(cx, |view, _| view.set_processes(processes));
    cx.notify();
}

fn on_process_count_sampled(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    count: u32,
    cx: &mut Context<AppShell>,
) {
    if shell.state.active_monitor_target() != Some((tab_id, connection_id)) {
        return;
    }
    shell
        .state
        .monitor
        .update(cx, |view, _| view.set_process_total(count));
    cx.notify();
}

fn on_processes_failed(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    error: AppError,
    cx: &mut Context<AppShell>,
) {
    if shell.state.active_monitor_target() != Some((tab_id, connection_id)) {
        return;
    }
    shell.state.monitor.update(cx, |view, _| {
        view.set_error(Some(crate::palette::localized_error(&error)));
    });
    cx.notify();
}

fn on_process_killed(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    pid: u32,
    error: Option<AppError>,
    cx: &mut Context<AppShell>,
) {
    if shell.state.active_monitor_target() != Some((tab_id, connection_id)) {
        return;
    }
    match error {
        Some(error) => {
            shell.state.monitor.update(cx, |view, _| {
                view.set_error(Some(crate::palette::localized_error(&error)));
            });
        }
        None => {
            shell
                .state
                .monitor
                .update(cx, |view, _| view.set_error(None));
            shell.state.status = Some(rust_i18n::t!("monitor.kill_sent", pid = pid).to_string());
            crate::monitor::sample_processes(shell, false, cx);
        }
    }
    cx.notify();
}

/// 记录完整指标时序快照。
pub(crate) fn monitor_sampled(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    processes: Vec<ProcessInfo>,
    cx: &mut Context<AppShell>,
) {
    on_processes_sampled(shell, tab_id, connection_id, processes, cx);
}

/// 折叠态的进程总数采样落地。
pub(crate) fn monitor_count_sampled(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    count: u32,
    cx: &mut Context<AppShell>,
) {
    on_process_count_sampled(shell, tab_id, connection_id, count, cx);
}

pub(crate) fn monitor_failed(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    error: AppError,
    cx: &mut Context<AppShell>,
) {
    on_processes_failed(shell, tab_id, connection_id, error, cx);
}

pub(crate) fn monitor_killed(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    pid: u32,
    error: Option<AppError>,
    cx: &mut Context<AppShell>,
) {
    on_process_killed(shell, tab_id, connection_id, pid, error, cx);
}
