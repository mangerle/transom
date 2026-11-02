//! 容器事件分发：按事件类别交给各自处理函数。

mod result;
mod terminal;

use super::*;

impl AppShell {
    /// 容器事件分发：先按请求 ID 找到工作区，再更新对应字段。
    pub(crate) fn dispatch_container_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::Connected { tab_id, info } => {
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Connected;
                    workspace.info = Some(*info);
                    workspace.notice = None;
                }
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Connected;
                }
                self.refresh_container_section(tab_id);
            }
            ContainerAppEvent::ConnectFailed { tab_id, error } => {
                let message = localized_error(&error);
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Failed(message.clone());
                }
                // 连接失败既要让工作区停在失败态（重试按钮在那里），也要在右上角
                // 立刻说一声：标签可能不在前台，只靠面板状态用户看不到出了事。
                self.show_toast(message, false, window, cx);
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Failed(error);
                }
            }
            ContainerAppEvent::EnginesDiscovered {
                engines, failures, ..
            } => {
                self.state.containers.discovering = false;
                self.state.containers.discovery = Some(ContainerDiscovery {
                    candidates: engines,
                    failures,
                });
                cx.notify();
            }
            ContainerAppEvent::TestFinished {
                request_id,
                info,
                error,
            } => {
                if let Some(form) = self.state.containers.form.as_mut()
                    && form.test_request == Some(request_id)
                {
                    form.test_request = None;
                    match (info, error) {
                        (Some(info), _) => {
                            form.test_ok = true;
                            form.test_status = Some(
                                rust_i18n::t!(
                                    "container.form.test_ok",
                                    platform = info.platform,
                                    version = info.version
                                )
                                .to_string(),
                            );
                        }
                        (None, Some(error)) => {
                            form.test_ok = false;
                            form.test_status = Some(localized_error(&error));
                        }
                        (None, None) => {
                            form.test_ok = false;
                            form.test_status =
                                Some(rust_i18n::t!("container.form.test_failed").to_string());
                        }
                    }
                }
            }
            ContainerAppEvent::Disconnected { tab_id } => {
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Disconnected;
                }
            }
            ContainerAppEvent::ContainersListed {
                request_id,
                containers,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                let mut inspect_first = None;
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.compose = transom_core::compose_projects(&containers);
                    workspace.containers = containers;
                    workspace.busy = false;
                    // 只有容器/编排分类的选中项是容器 ID；在卷等其他分类下顺手
                    // 拉一次容器列表时，不能把当前的选中项判成"无效"清掉。
                    let id_is_container_scoped = matches!(
                        workspace.section,
                        ContainerSection::Containers | ContainerSection::Compose
                    );
                    if id_is_container_scoped {
                        let valid = workspace.selected.as_deref().is_some_and(|id| {
                            workspace.containers.iter().any(|item| item.id == id)
                        });
                        if !valid {
                            workspace.selected = None;
                            workspace.details = None;
                        }
                    }
                    // 跨分类跳转等着列表到位再按名字定位，避免用户点了两次。
                    if let Some(name) = workspace.pending_select.take() {
                        inspect_first = workspace
                            .containers
                            .iter()
                            .find(|item| item.name == name)
                            .map(|item| item.id.clone());
                    }
                    if workspace.section == ContainerSection::Containers
                        && workspace.selected.is_none()
                        && inspect_first.is_none()
                        && let Some(first) = workspace.containers.first()
                    {
                        inspect_first = Some(first.id.clone());
                    }
                }
                if let Some(id) = inspect_first {
                    self.select_container_item(tab_id, id);
                }
            }
            ContainerAppEvent::ContainerInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.id.clone());
                    workspace.details = Some(*details);
                }
            }
            ContainerAppEvent::ImagesListed { request_id, images } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.images = images;
                    workspace.busy = false;
                    let valid = workspace
                        .selected
                        .as_deref()
                        .is_some_and(|id| workspace.images.iter().any(|item| item.id == id));
                    if !valid {
                        workspace.selected = workspace.images.first().map(|item| item.id.clone());
                        workspace.image_details = None;
                    }
                }
            }
            ContainerAppEvent::ImageInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.id.clone());
                    workspace.image_details = Some(*details);
                }
            }
            ContainerAppEvent::VolumesListed {
                request_id,
                volumes,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.volumes = volumes;
                    workspace.busy = false;
                    let valid = workspace
                        .selected
                        .as_deref()
                        .is_some_and(|name| workspace.volumes.iter().any(|item| item.name == name));
                    if !valid {
                        workspace.selected =
                            workspace.volumes.first().map(|item| item.name.clone());
                        workspace.volume_details = None;
                    }
                }
            }
            ContainerAppEvent::VolumeInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.summary.name.clone());
                    workspace.volume_details = Some(*details);
                }
            }
            ContainerAppEvent::NetworksListed {
                request_id,
                networks,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.networks = networks;
                    workspace.busy = false;
                    let valid = workspace
                        .selected
                        .as_deref()
                        .is_some_and(|id| workspace.networks.iter().any(|item| item.id == id));
                    if !valid {
                        workspace.selected = workspace.networks.first().map(|item| item.id.clone());
                        workspace.network_details = None;
                    }
                }
            }
            ContainerAppEvent::NetworkInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.summary.id.clone());
                    workspace.network_details = Some(*details);
                }
            }
            ContainerAppEvent::ComposeProjectsListed {
                request_id,
                projects,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.compose = projects;
                    workspace.busy = false;
                }
            }
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
                if let Some(workspace) = self.state.container_workspace_mut(tab_id)
                    && workspace.logs_request == Some(request_id)
                {
                    workspace.append_logs(&bytes);
                }
            }
            ContainerAppEvent::LogsClosed { request_id } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id)
                    && workspace.logs_request == Some(request_id)
                {
                    workspace.logs_request = None;
                    workspace.logs_follow = false;
                }
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
                if let Some(workspace) = self.state.container_workspace_mut(tab_id)
                    && workspace.stats_request == Some(request_id)
                {
                    let due = workspace
                        .stats_last_append
                        .is_none_or(|last| last.elapsed() >= workspace.stats_interval.interval());
                    if due {
                        workspace.stats_last_append = Some(std::time::Instant::now());
                        workspace.stats_history.push_back(stats.clone());
                        while workspace.stats_history.len() > STATS_HISTORY_LIMIT {
                            workspace.stats_history.pop_front();
                        }
                        workspace.stats = Some(stats);
                    }
                    workspace.stats_error = None;
                }
            }
            ContainerAppEvent::StatsUnavailable { request_id, error } => {
                let Some(tab_id) = self.state.containers.tab_of(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id)
                    && workspace.stats_request == Some(request_id)
                {
                    workspace.stats_error = Some(
                        rust_i18n::t!("container.stats.reconnecting", error = error).to_string(),
                    );
                }
            }
            ContainerAppEvent::ProcessesListed {
                request_id,
                processes,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.stats_processes_request = None;
                    workspace.stats_processes_loading = false;
                    workspace.stats_processes = processes;
                    workspace.stats_processes_error = None;
                }
            }
            ContainerAppEvent::DirListed {
                request_id,
                path,
                entries,
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id)
                    && workspace.files.request == Some(request_id)
                {
                    log::info!("容器目录已加载，路径: {path}，条目: {}", entries.len());
                    workspace.files.request = None;
                    workspace.files.path = path;
                    workspace.files.entries = entries;
                    workspace.files.loading = false;
                    workspace.files.scroll.set_offset(point(px(0.0), px(0.0)));
                }
            }
            ContainerAppEvent::FilePreview {
                request_id,
                path,
                content,
                truncated,
                ..
            } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.files.preview = Some((path, content, truncated));
                }
            }
            ContainerAppEvent::TransferProgress {
                request_id,
                transferred,
                total,
            } => {
                let Some(tab_id) = self.state.containers.tab_of(request_id) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.notice = Some(match total {
                        Some(total) if total > 0 => format!(
                            "{} / {}",
                            crate::format::format_size(transferred),
                            crate::format::format_size(total)
                        ),
                        _ => crate::format::format_size(transferred),
                    });
                }
            }
            ContainerAppEvent::VolumesPruned { request_id, report } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                let target = ContainerPruneTarget::Volumes;
                let message = if report.deleted.is_empty() {
                    rust_i18n::t!(target.none_key()).to_string()
                } else {
                    rust_i18n::t!(target.done_key(), count = report.deleted.len()).to_string()
                };
                self.show_toast(message, true, window, cx);
                self.refresh_container_section(tab_id);
            }
            ContainerAppEvent::ImagesPruned { request_id, report } => {
                let Some(tab_id) = self.state.containers.take_tab(request_id) else {
                    return;
                };
                let target = ContainerPruneTarget::Images;
                let message = if report.deleted.is_empty() {
                    rust_i18n::t!(target.none_key()).to_string()
                } else {
                    rust_i18n::t!(target.done_key(), count = report.deleted.len()).to_string()
                };
                self.show_toast(message, true, window, cx);
                self.refresh_container_section(tab_id);
            }
            ContainerAppEvent::ActionFinished { request_id, error } => {
                self.on_container_action_finished(request_id, error, window, cx)
            }
            ContainerAppEvent::TransferFinished { request_id, error } => {
                self.on_container_transfer_finished(request_id, error, window, cx)
            }
            ContainerAppEvent::Failed { request_id, error } => {
                self.on_container_failed(request_id, error, window, cx)
            }
        }
        cx.notify();
    }
}
