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
            crate::app::monitor::after_monitor_collapse(shell, *collapse, *expanded_processes, cx);
        }
        MonitorPanelEvent::RestartSampling(interval) => {
            crate::app::monitor::restart_monitor_task(shell, *interval, cx);
        }
        MonitorPanelEvent::SampleProcesses { collapsed } => {
            crate::app::monitor::sample_processes(shell, *collapsed, cx);
        }
        MonitorPanelEvent::TraceProcess(pid) => {
            crate::app::monitor::trace_process_in_terminal(shell, *pid, cx);
        }
        MonitorPanelEvent::KillProcess { pid, force } => {
            crate::app::monitor::send_kill_process(shell, *pid, *force, cx);
        }
        MonitorPanelEvent::RestartService(unit) => {
            crate::app::monitor::restart_systemd_service(shell, unit, cx);
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
        crate::app::monitor::sample_processes(
            shell,
            shell.state.monitor.read(cx).processes_collapsed(),
            cx,
        );
        crate::app::monitor::ensure_monitor_task(
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
    crate::app::monitor::ensure_monitor_task(shell, interval, cx);
    cx.notify();
}

/// 切换采样频率档位（1s/3s/10s），并重启轮询任务。
///
/// 仅用于面板之外（设置页）的调用点；面板内点击走
/// [`crate::app::monitor::restart_monitor_task`]。
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
        crate::app::monitor::ensure_monitor_task(
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
    crate::app::monitor::sample_processes(
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
        crate::app::monitor::sample_processes(shell, false, cx);
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
    crate::app::monitor::send_terminal_input(shell, &cmd, cx);
}

/// 在终端中重启崩溃的 Systemd 服务。
pub(crate) fn restart_systemd_service(
    shell: &mut AppShell,
    unit: &str,
    cx: &mut Context<AppShell>,
) {
    let cmd = format!("systemctl restart {unit}\r");
    crate::app::monitor::send_terminal_input(shell, &cmd, cx);
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
            crate::app::monitor::sample_processes(shell, false, cx);
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

/// 面板已改好宽度后，外壳侧同步持久化并让分栏重新分配尺寸。
///
/// 宽度状态由面板写入（见 `MonitorView::set_drawer_width`）；面板点击回调
/// 执行期间实体被租借，这里只做设置落盘与 `ResizableState` 调整。
pub(crate) fn apply_monitor_drawer_width(
    shell: &mut AppShell,
    new_width: f32,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    update_app_settings(cx, |s| s.monitor.drawer_width = new_width);
    shell.persist_layout_settings(cx);
    let panels_state = if app_settings(cx).window.sftp_panel_side.is_left() {
        &shell.state.workspace_panels_left
    } else {
        &shell.state.workspace_panels
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
pub(crate) fn maximize_monitor_to_tab(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    // 1. 寻找当前关联的主机会话 TabId
    let active_tab_id = shell.state.workspace.active_tab;
    let target_tab_id = active_tab_id
        .and_then(|id| shell.state.workspace.tab(id))
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
            shell
                .state
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
    if let Some(existing) = shell.state.workspace.tabs.iter().find(|t| {
        if let TabContent::Monitor(tid) = t.content {
            tid == target_tab_id
        } else {
            false
        }
    }) {
        let existing_id = existing.id;
        shell.activate_tab(existing_id, window, cx);
        cx.notify();
        return;
    }

    // 3. 构建全屏监控独立标签页
    let Some(target_tab) = shell.state.workspace.tab(target_tab_id) else {
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

    shell
        .state
        .workspace
        .create_group(monitor_tab_id, target_session_id);
    shell.state.workspace.tabs.push(new_tab);
    shell.activate_tab(monitor_tab_id, window, cx);
    cx.notify();
}

/// 将全屏监控独立标签页收回并停靠为右侧抽屉。
pub(crate) fn dock_monitor_to_drawer(
    shell: &mut AppShell,
    tab_id: TabId,
    target_tab_id: TabId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    // 抽屉的开合由面板在调用前写好；这里只做标签切换与关闭。
    // 2. 切回原会话标签页
    if shell.state.workspace.tab(target_tab_id).is_some() {
        shell.activate_tab(target_tab_id, window, cx);
    }

    // 3. 关闭全屏监控 Tab
    shell.close_pane(tab_id, window, cx);
    cx.notify();
}
