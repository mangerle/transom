//! 监控看板的采样生命周期、轮询调度与进程信号发送。

use super::types::PROCESS_POLL_INTERVAL;
use crate::app::AppShell;
use gpui_kit::{Context, Window};
use transom_core::{AppCommand, AppError, ConnectionId, InterfaceRate, ProcessInfo, TabId};

impl AppShell {
    /// 活动栏入口：打开/关闭监控抽屉，打开时立即拉取一轮进程快照。
    pub(crate) fn toggle_monitor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.monitor.open = !self.monitor.open;
        if self.monitor.open {
            self.monitor.history_tab = None;
            self.monitor.error = None;
            self.sample_processes(cx);
            self.ensure_monitor_task(cx);
        } else {
            self.monitor.processes.clear();
        }
        cx.notify();
    }

    /// 保证进程轮询任务已启动；任务常驻但在面板关闭时不发命令。
    fn ensure_monitor_task(&mut self, cx: &mut Context<Self>) {
        if self.monitor_task.is_some() {
            return;
        }
        self.monitor_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROCESS_POLL_INTERVAL).await;
                if this
                    .update(cx, |this, cx| this.sample_processes_if_open(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn sample_processes_if_open(&mut self, cx: &mut Context<Self>) {
        if !self.monitor.open {
            return;
        }
        self.sample_processes(cx);
    }

    pub(crate) fn sample_processes(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        self.send_command(AppCommand::SampleProcesses {
            tab_id,
            connection_id,
        });
        cx.notify();
    }

    /// 当前活动标签的进程采样目标；只有 SSH 与本地终端支持。
    fn active_monitor_target(&self) -> Option<(TabId, ConnectionId)> {
        let tab_id = self.active_tab?;
        let tab = self.tabs.iter().find(|tab| tab.id == tab_id)?;
        if !matches!(
            tab.session.connection,
            transom_core::Connection::Ssh(_) | transom_core::Connection::Local(_)
        ) {
            return None;
        }
        Some((tab_id, tab.connection_id?))
    }

    pub(crate) fn send_kill_process(&mut self, pid: u32, force: bool, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        self.send_command(AppCommand::KillProcess {
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

    pub(crate) fn monitor_record_network(&mut self, tab_id: TabId, interfaces: &[InterfaceRate]) {
        if self.monitor.open {
            self.monitor.record_network(tab_id, interfaces);
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
}
