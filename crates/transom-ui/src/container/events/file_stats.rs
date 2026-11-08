//! 日志、统计、文件浏览与传输进度。

use super::super::*;

impl AppShell {
    pub(super) fn handle_file_stats_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::ExecReady {
                tab_id,
                request_id,
                io,
                exit,
            } => self.on_container_exec_ready(tab_id, request_id, *io, exit, window, cx),
            ContainerAppEvent::LogsChunk { request_id, bytes } => {
                let Some(tab_id) = self.state.containers.tab_of(request_id) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    if workspace.logs_request == Some(request_id) {
                        workspace.append_logs(&bytes);
                    }
                });
            }
            ContainerAppEvent::LogsClosed { request_id } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    if workspace.logs_request == Some(request_id) {
                        workspace.logs_request = None;
                        workspace.logs_follow = false;
                    }
                });
            }
            ContainerAppEvent::StatsUpdated {
                request_id,
                container: _,
                stats,
            } => {
                let Some(tab_id) = self.state.containers.tab_of(request_id) else {
                    return;
                };
                log::debug!("收到容器统计更新，请求: {request_id}");
                self.with_container_mut(tab_id, cx, |workspace| {
                    if workspace.stats_request != Some(request_id) {
                        return;
                    }
                    let mut stats = stats;
                    if stats.host_load.is_none()
                        && let Some(prev) = &workspace.stats
                        && prev.host_load.is_some()
                    {
                        stats.host_load = prev.host_load;
                    }
                    workspace.stats_high_cpu = if stats.cpu_percent >= 90.0 {
                        workspace.stats_high_cpu.saturating_add(1)
                    } else {
                        0
                    };
                    workspace.stats_high_memory = if stats.memory_percent >= 90.0 {
                        workspace.stats_high_memory.saturating_add(1)
                    } else {
                        0
                    };
                    let high_cpu = workspace.stats_high_cpu >= 3;
                    let high_memory = workspace.stats_high_memory >= 3;
                    let alert_due = workspace
                        .stats_alert_at
                        .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(300));
                    if (high_cpu || high_memory) && alert_due {
                        workspace.stats_alert_at = Some(std::time::Instant::now());
                        workspace.notice = Some(if high_cpu {
                            rust_i18n::t!("container.stats.alert_cpu").to_string()
                        } else {
                            rust_i18n::t!("container.stats.alert_memory").to_string()
                        });
                    }
                    workspace.stats = Some(stats.clone());
                    let due = workspace
                        .stats_last_append
                        .is_none_or(|last| last.elapsed() >= workspace.stats_interval.interval());
                    if due {
                        workspace.stats_last_append = Some(std::time::Instant::now());
                        workspace.stats_history.push_back(stats);
                        while workspace.stats_history.len() > STATS_HISTORY_LIMIT {
                            workspace.stats_history.pop_front();
                        }
                    }
                    workspace.stats_error = None;
                });
            }
            ContainerAppEvent::StatsUnavailable { request_id, error } => {
                let Some(tab_id) = self.state.containers.tab_of(request_id) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    if workspace.stats_request == Some(request_id) {
                        workspace.stats_error = Some(
                            rust_i18n::t!("container.stats.reconnecting", error = error)
                                .to_string(),
                        );
                    }
                });
            }
            ContainerAppEvent::DirListed {
                request_id,
                path,
                entries,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    if workspace.files.request == Some(request_id) {
                        log::info!("容器目录已加载，路径: {path}，条目: {}", entries.len());
                        workspace.files.request = None;
                        workspace.files.path = path;
                        workspace.files.entries = entries;
                        workspace.files.loading = false;
                        workspace.files.scroll.set_offset(point(px(0.0), px(0.0)));
                    }
                });
            }
            ContainerAppEvent::FilePreview {
                request_id,
                path,
                content,
                truncated,
                binary,
                ..
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.files.preview = Some((path, content, truncated));
                    workspace.files.preview_binary = binary;
                    if binary {
                        workspace.files.preview_editor = None;
                    }
                });
            }
            ContainerAppEvent::TransferProgress {
                request_id,
                transferred,
                total,
            } => {
                let Some(tab_id) = self.state.containers.tab_of(request_id) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.notice = Some(match total {
                        Some(total) if total > 0 => format!(
                            "{} / {}",
                            crate::format::format_size(transferred),
                            crate::format::format_size(total)
                        ),
                        _ => crate::format::format_size(transferred),
                    });
                });
            }
            ContainerAppEvent::BuildProgress { request_id, line } => {
                if let Some(tab_id) = self.state.containers.tab_of(request_id) {
                    self.with_container_mut(tab_id, cx, |workspace| {
                        workspace.notice = line.lines().last().map(str::to_string);
                        if let Some(b) = workspace
                            .build_dialog
                            .as_mut()
                            .filter(|b| b.request_id == Some(request_id))
                        {
                            b.logs.extend(line.lines().map(str::to_string));
                            if b.logs.len() > crate::container::types::BUILD_LOG_LIMIT {
                                let excess =
                                    b.logs.len() - crate::container::types::BUILD_LOG_LIMIT;
                                b.logs.drain(..excess);
                            }
                        }
                    });
                }
            }
            _ => {}
        }
    }
}
