//! 容器统计采样：启动/停止采样、时间档位与图表形态切换。

use super::*;

impl AppShell {
    pub(crate) fn ensure_container_stats(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
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
        workspace.stats = None;
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
        // 兜底看门狗：请求若因链路异常一直没有回音，12 秒后给出可重试的错误，
        // 避免界面永远停在「正在采样统计…」。
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(12))
                .await;
            let _ = this.update(cx, |this, cx| {
                let pending = this
                    .state
                    .container_workspace(tab_id)
                    .is_some_and(|workspace| workspace.stats_request == Some(request_id));
                if !pending {
                    return;
                }
                if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                    workspace.stats_request = None;
                    workspace.stats_error =
                        Some(rust_i18n::t!("container.stats.timeout").to_string());
                }
                this.cancel_container_request(tab_id, request_id);
                cx.notify();
            });
        })
        .detach();
    }

    /// 切换采样档位：历史与速率口径都依赖间隔，换档时清空历史重新累积。
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
        workspace.stats_history.clear();
        let stats_request = workspace.stats_request.take();
        // 统计页始终采样；换档位时按新间隔重启轮询，不在统计页则只记档位。
        let should_poll = workspace.detail_tab == ContainerDetailTab::Stats;
        if let Some(request_id) = stats_request {
            self.cancel_container_request(tab_id, request_id);
        }
        if should_poll {
            self.ensure_container_stats(tab_id, cx);
        }
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
        let Some(container) = workspace.selected_container().map(|item| item.name.clone()) else {
            return;
        };
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
}
