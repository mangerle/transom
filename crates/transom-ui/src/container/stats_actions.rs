//! 容器统计采样：启动/停止采样、时间档位与图表形态切换。

use super::*;
use crate::app::spawn_blocking_local;
use crate::monitor::{SampleIntervalMode, SparklineChartType};
use std::fmt::Write as _;
use transom_core::ContainerStats;

impl ContainerWorkspace {
    /// 把当前统计历史导出为 CSV，便于离线分析或贴到工单。
    ///
    /// 写盘交由阻塞线程池完成：统计历史可能很长，同步写盘会卡住渲染帧。
    /// 结果写入工作区 `notice`，由外壳的 `flush_container_notice` 统一转成通知，
    /// 因此这里不需要窗口句柄。
    pub(crate) fn export_container_stats_csv(&mut self, cx: &mut Context<Self>) {
        let Some(container) = self.state.selected_container_name() else {
            return;
        };
        let history: Vec<ContainerStats> = self.state.stats_history.iter().cloned().collect();
        let interval = self.state.stats_interval.interval();
        if history.is_empty() {
            self.state.notice = Some(rust_i18n::t!("container.stats.export_empty").to_string());
            cx.notify();
            return;
        }
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(format!("{container}-stats.csv"))
            .save_file()
        else {
            return;
        };
        let write_path = local_path.clone();
        cx.spawn(async move |this, cx| {
            let outcome = spawn_blocking_local(move || {
                let csv = stats_history_csv(&container, &history, interval);
                std::fs::write(&write_path, csv)
                    .map_err(|error| error.to_string())
                    .map(|_| write_path)
            })
            .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.state.notice = Some(match outcome {
                    Some(Ok(path)) => rust_i18n::t!(
                        "container.stats.export_done",
                        path = path.display().to_string()
                    )
                    .to_string(),
                    Some(Err(error)) => {
                        log::error!(
                            "导出容器统计 CSV 失败，路径: {}，原因: {error}",
                            local_path.display()
                        );
                        rust_i18n::t!("container.stats.export_failed", error = error).to_string()
                    }
                    None => {
                        log::error!(
                            "导出容器统计 CSV 失败，路径: {}，原因: 后台写入线程不可用",
                            local_path.display()
                        );
                        rust_i18n::t!("container.stats.export_thread_failed").to_string()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// 确保当前选中容器的统计流已经建立；已有在途请求时不重复发起。
    pub(crate) fn ensure_container_stats(&mut self) {
        if self.state.stats_request.is_some() {
            return;
        }
        let Some((container, running)) = self
            .state
            .selected_container()
            .map(|container| (container.name.clone(), container.state.is_running()))
        else {
            return;
        };
        // 停掉的容器采不到统计（引擎直接 409）；先把话说清楚，别让页面空等。
        if !running {
            self.state.stats = None;
            self.state.stats_error = Some(rust_i18n::t!("container.stats.not_running").to_string());
            return;
        }
        let interval_ms = self.state.stats_interval.interval().as_millis() as u64;
        let request_id = ContainerRequestId::new();
        self.state.stats_request = Some(request_id);
        self.state.stats_error = None;
        log::debug!(
            "请求容器统计，标签: {}，容器: {container}，间隔: {interval_ms}ms，请求: {request_id}",
            self.tab_id
        );
        self.send_tracked(
            request_id,
            ContainerCommand::Stats {
                tab_id: self.tab_id,
                request_id,
                container,
                enabled: true,
                interval_ms,
            },
        );
    }

    /// 切换采样档位：保留当前值与已绘制的历史，只按新间隔重启后续采样。
    pub(crate) fn set_container_stats_interval(&mut self, interval: SampleIntervalMode) {
        if self.state.stats_interval == interval {
            return;
        }
        self.state.stats_interval = interval;
    }

    /// 手动重启统计流；后台断线会自动重连，这里用于用户主动重试。
    pub(crate) fn restart_container_stats(&mut self) {
        self.state.stats_last_append = None;
        if let Some(request_id) = self.state.stats_request.take() {
            self.cancel_container_request(request_id);
        }
        self.ensure_container_stats();
    }

    /// 拉取容器进程列表（一次性请求，可手动刷新）。
    pub(crate) fn ensure_container_processes(&mut self) {
        if self.state.stats_processes_request.is_some() {
            return;
        }
        let Some((container, running)) = self
            .state
            .selected_container()
            .map(|container| (container.name.clone(), container.state.is_running()))
        else {
            return;
        };
        // 停掉的容器查不到进程（引擎直接报 409 Conflict 操作冲突）；直接置空，避免发出无效请求。
        if !running {
            self.state.stats_processes.clear();
            self.state.stats_processes_loading = false;
            self.state.stats_processes_error = None;
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.stats_processes_request = Some(request_id);
        self.state.stats_processes_loading = true;
        self.state.stats_processes_error = None;
        self.send_tracked(
            request_id,
            ContainerCommand::ListProcesses {
                tab_id: self.tab_id,
                request_id,
                container,
            },
        );
    }

    /// 切换统计走势图形态：与主机监控共用同一套折线/柱状组件。
    pub(crate) fn toggle_container_stats_chart(&mut self) {
        self.state.stats_chart = match self.state.stats_chart {
            SparklineChartType::Line => SparklineChartType::Bar,
            SparklineChartType::Bar => SparklineChartType::Line,
        };
    }

    /// 切换统计看板端口映射表格折叠状态。
    pub(crate) fn toggle_container_stats_ports(&mut self) {
        self.state.stats_ports_collapsed = !self.state.stats_ports_collapsed;
    }

    /// 切换统计看板容器进程表格折叠状态。
    pub(crate) fn toggle_container_stats_processes(&mut self) {
        self.state.stats_processes_collapsed = !self.state.stats_processes_collapsed;
    }
}

/// 生成统计历史 CSV；时间戳按采样间隔从当前时间反推。
fn stats_history_csv(
    container: &str,
    history: &[ContainerStats],
    interval: std::time::Duration,
) -> String {
    let mut csv = String::with_capacity(history.len() * 128 + 128);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let step = interval.as_secs().max(1);
    let _ = writeln!(
        csv,
        "container,timestamp,cpu_percent,cpu_count,memory_used,memory_limit,memory_percent,network_rx,network_tx,block_read,block_write,pids"
    );
    for (index, sample) in history.iter().enumerate() {
        let age = (history.len().saturating_sub(index + 1)) as u64;
        let timestamp = now.saturating_sub(age.saturating_mul(step));
        let _ = writeln!(
            csv,
            "{container},{timestamp},{:.2},{},{},{},{:.2},{},{},{},{},{}",
            sample.cpu_percent,
            sample.cpu_count,
            sample.memory_used,
            sample.memory_limit,
            sample.memory_percent,
            sample.network_rx,
            sample.network_tx,
            sample.block_read,
            sample.block_write,
            sample.pids
        );
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::stats_history_csv;
    use transom_core::ContainerStats;

    #[test]
    fn stats_csv_has_header_and_rows() {
        let history = vec![
            ContainerStats {
                cpu_percent: 12.5,
                memory_used: 1024,
                ..ContainerStats::default()
            },
            ContainerStats {
                cpu_percent: 25.0,
                memory_used: 2048,
                ..ContainerStats::default()
            },
        ];
        let csv = stats_history_csv("web-1", &history, std::time::Duration::from_secs(3));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("container,timestamp"));
        assert!(lines[1].starts_with("web-1,"));
        assert!(lines[1].contains(",12.50,"));
    }
}
