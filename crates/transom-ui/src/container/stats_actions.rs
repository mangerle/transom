//! 容器统计采样：启动/停止采样、时间档位与图表形态切换。

use super::*;
use crate::app::spawn_blocking_local;
use std::fmt::Write as _;
use transom_core::ContainerStats;

impl AppShell {
    /// 把当前统计历史导出为 CSV，便于离线分析或贴到工单。
    pub(crate) fn export_container_stats_csv(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((container, history, interval)) = self
            .with_container(tab_id, cx, |workspace| {
                workspace.selected_container_name().map(|container| {
                    let history: Vec<ContainerStats> =
                        workspace.stats_history.iter().cloned().collect();
                    (container, history, workspace.stats_interval.interval())
                })
            })
            .flatten()
        else {
            return;
        };
        if history.is_empty() {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.notice = Some(rust_i18n::t!("container.stats.export_empty").to_string());
            });
            cx.notify();
            return;
        }
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(format!("{container}-stats.csv"))
            .save_file()
        else {
            return;
        };
        let container_for_task = container.clone();
        cx.spawn(async move |this, cx| {
            let outcome = spawn_blocking_local(move || {
                let csv = stats_history_csv(&container_for_task, &history, interval);
                std::fs::write(&local_path, csv)
                    .map_err(|error| error.to_string())
                    .map(|_| local_path)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.with_container_mut(tab_id, cx, |workspace| {
                    workspace.notice = Some(match outcome {
                        Some(Ok(path)) => rust_i18n::t!(
                            "container.stats.export_done",
                            path = path.display().to_string()
                        )
                        .to_string(),
                        Some(Err(error)) => {
                            rust_i18n::t!("container.stats.export_failed", error = error)
                                .to_string()
                        }
                        None => rust_i18n::t!("container.stats.export_thread_failed").to_string(),
                    });
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn ensure_container_stats(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((container, running, interval)) = self
            .with_container(tab_id, cx, |workspace| {
                if workspace.stats_request.is_some() {
                    return None;
                }
                let container = workspace.selected_container()?;
                Some((
                    container.name.clone(),
                    container.state.is_running(),
                    workspace.stats_interval.interval(),
                ))
            })
            .flatten()
        else {
            return;
        };
        // 停掉的容器采不到统计（引擎直接 409）；先把话说清楚，别让页面空等。
        if !running {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.stats = None;
                workspace.stats_error =
                    Some(rust_i18n::t!("container.stats.not_running").to_string());
            });
            return;
        }
        let interval_ms = interval.as_millis() as u64;
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.stats_request = Some(request_id);
            workspace.stats_error = None;
        });
        log::debug!(
            "请求容器统计，标签: {tab_id}，容器: {container}，间隔: {interval_ms}ms，请求: {request_id}"
        );
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::Stats {
                        tab_id,
                        request_id,
                        container,
                        enabled: true,
                        interval_ms,
                    },
                );
            });
        }
    }

    /// 切换采样档位：保留当前值与已绘制的历史，只按新间隔重启后续采样。
    pub(crate) fn set_container_stats_interval(
        &mut self,
        tab_id: TabId,
        interval: crate::monitor::SampleIntervalMode,
        cx: &mut Context<Self>,
    ) {
        let changed = self.with_container_mut(tab_id, cx, |workspace| {
            if workspace.stats_interval == interval {
                return false;
            }
            workspace.stats_interval = interval;
            true
        });
        if changed == Some(true) {
            cx.notify();
        }
    }

    /// 手动重启统计流；后台断线会自动重连，这里用于用户主动重试。
    pub(crate) fn restart_container_stats(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let request_id = self
            .with_container_mut(tab_id, cx, |workspace| {
                workspace.stats_last_append = None;
                workspace.stats_request.take()
            })
            .flatten();
        if let Some(request_id) = request_id
            && let Some(workspace) = self.state.container_workspace(tab_id)
        {
            workspace.update(cx, |workspace, _| {
                workspace.cancel_container_request(request_id)
            });
        }
        self.ensure_container_stats(tab_id, cx);
        cx.notify();
    }

    /// 拉取容器进程列表（一次性请求，可手动刷新）。
    pub(crate) fn ensure_container_processes(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((container, running)) = self
            .with_container(tab_id, cx, |workspace| {
                if workspace.stats_processes_request.is_some() {
                    return None;
                }
                let container = workspace.selected_container()?;
                Some((container.name.clone(), container.state.is_running()))
            })
            .flatten()
        else {
            return;
        };
        // 停掉的容器查不到进程（引擎直接报 409 Conflict 操作冲突）；直接置空，避免发出无效请求。
        if !running {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.stats_processes.clear();
                workspace.stats_processes_loading = false;
                workspace.stats_processes_error = None;
            });
            return;
        }
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.stats_processes_request = Some(request_id);
            workspace.stats_processes_loading = true;
            workspace.stats_processes_error = None;
        });
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::ListProcesses {
                        tab_id,
                        request_id,
                        container,
                    },
                );
            });
        }
    }

    /// 切换统计走势图形态：与主机监控共用同一套折线/柱状组件。
    pub(crate) fn toggle_container_stats_chart(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.stats_chart = match workspace.stats_chart {
                crate::monitor::SparklineChartType::Line => crate::monitor::SparklineChartType::Bar,
                crate::monitor::SparklineChartType::Bar => crate::monitor::SparklineChartType::Line,
            };
        });
        cx.notify();
    }

    /// 切换统计看板端口映射表格折叠状态。
    pub(crate) fn toggle_container_stats_ports(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.stats_ports_collapsed = !workspace.stats_ports_collapsed;
        });
        cx.notify();
    }

    /// 切换统计看板容器进程表格折叠状态。
    pub(crate) fn toggle_container_stats_processes(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.stats_processes_collapsed = !workspace.stats_processes_collapsed;
        });
        cx.notify();
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
