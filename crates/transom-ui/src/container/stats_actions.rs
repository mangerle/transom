//! 容器统计采样：启动/停止采样、时间档位与图表形态切换。

use super::*;

impl AppShell {
    pub(crate) fn ensure_container_stats(&mut self, tab_id: TabId, _cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.stats_request.is_some() {
            return;
        }
        let Some(container) = workspace.selected_container() else {
            return;
        };
        // 停掉的容器采不到统计（引擎直接 409）；先把话说清楚，别让页面空等。
        if !container.state.is_running() {
            workspace.stats = None;
            workspace.stats_error = Some(rust_i18n::t!("container.stats.not_running").to_string());
            return;
        }
        let container = container.name.clone();
        let interval_ms = workspace.stats_interval.interval().as_millis() as u64;
        let request_id = ContainerRequestId::new();
        workspace.stats_request = Some(request_id);
        workspace.stats_error = None;
        log::debug!(
            "请求容器统计，标签: {tab_id}，容器: {container}，间隔: {interval_ms}ms，请求: {request_id}"
        );
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Stats {
                tab_id,
                request_id,
                container,
                enabled: true,
                interval_ms,
            },
        );
    }

    /// 切换采样档位：保留当前值与已绘制的历史，只按新间隔重启后续采样。
    pub(crate) fn set_container_stats_interval(
        &mut self,
        tab_id: TabId,
        interval: crate::monitor::SampleIntervalMode,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.stats_interval == interval {
            return;
        }
        workspace.stats_interval = interval;
        cx.notify();
    }

    /// 手动重启统计流；后台断线会自动重连，这里用于用户主动重试。
    pub(crate) fn restart_container_stats(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let request_id = self
            .state
            .container_workspace_mut(tab_id)
            .and_then(|workspace| {
                workspace.stats_last_append = None;
                workspace.stats_request.take()
            });
        if let Some(request_id) = request_id {
            self.cancel_container_request(tab_id, request_id);
        }
        self.ensure_container_stats(tab_id, cx);
        cx.notify();
    }

    /// 拉取容器进程列表（一次性请求，可手动刷新）。
    pub(crate) fn ensure_container_processes(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.stats_processes_request.is_some() {
            return;
        }
        let Some(container) = workspace.selected_container() else {
            return;
        };
        // 停掉的容器查不到进程（引擎直接报 409 Conflict 操作冲突）；直接置空，避免发出无效请求。
        if !container.state.is_running() {
            workspace.stats_processes.clear();
            workspace.stats_processes_loading = false;
            workspace.stats_processes_error = None;
            return;
        }
        let container = container.name.clone();
        let request_id = ContainerRequestId::new();
        workspace.stats_processes_request = Some(request_id);
        workspace.stats_processes_loading = true;
        workspace.stats_processes_error = None;
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ListProcesses {
                tab_id,
                request_id,
                container,
            },
        );
    }

    /// 切换统计走势图形态：与主机监控共用同一套折线/柱状组件。
    pub(crate) fn toggle_container_stats_chart(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.stats_chart = match workspace.stats_chart {
                crate::monitor::SparklineChartType::Line => crate::monitor::SparklineChartType::Bar,
                crate::monitor::SparklineChartType::Bar => crate::monitor::SparklineChartType::Line,
            };
        }
        cx.notify();
    }

    /// 切换统计看板端口映射表格折叠状态。
    pub(crate) fn toggle_container_stats_ports(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.stats_ports_collapsed = !workspace.stats_ports_collapsed;
        }
        cx.notify();
    }

    /// 切换统计看板容器进程表格折叠状态。
    pub(crate) fn toggle_container_stats_processes(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.stats_processes_collapsed = !workspace.stats_processes_collapsed;
        }
        cx.notify();
    }
}
