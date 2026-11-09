//! 列表与详情响应写回实体的具名方法。
//!
//! 设计原理（Why）：事件处理只负责按 `request_id` 找到标签页，具体状态流转
//! 收敛为实体方法，避免在外壳闭包里拼装大段业务逻辑，也便于单元测试与复用。
//! 需要后续请求（如自动 inspect 第一项）时由实体直接投递，不再回交外壳。

use super::super::*;

/// 自动调度下一条下载任务所需的参数：任务 ID、工作区、镜像引用与仓库凭据。
type ScheduledPull = (
    String,
    TabId,
    String,
    Option<transom_core::ContainerRegistryAuth>,
);

/// 下载任务结束后的收尾数据：任务引用与下一条待启动命令。
type PullFinishedOutcome = (String, Option<ScheduledPull>);

impl ContainerWorkspace {
    /// 应用容器列表响应：重建 Compose 视图、修正选中项并自动 inspect。
    pub(crate) fn apply_containers_listed(
        &mut self,
        containers: Vec<transom_core::ContainerSummary>,
    ) {
        self.state.compose = transom_core::compose_projects(&containers);
        self.state.containers = containers;
        self.state.busy = false;
        // 只有容器/编排分类的选中项是容器 ID；在卷等其他分类下顺手拉一次容器
        // 列表时，不能把当前的选中项判成“无效”清掉。
        let id_is_container_scoped = matches!(
            self.state.section,
            ContainerSection::Containers | ContainerSection::Compose
        );
        if id_is_container_scoped {
            let valid = self
                .state
                .selected
                .as_deref()
                .is_some_and(|id| self.state.containers.iter().any(|item| item.id == id));
            if !valid {
                self.state.selected = None;
                self.state.details = None;
            }
        }
        // 跨分类跳转等着列表到位再按名字定位，避免用户点了两次。
        let mut inspect_first = self.state.pending_select.take().and_then(|name| {
            self.state
                .containers
                .iter()
                .find(|item| item.name == name)
                .map(|item| item.id.clone())
        });
        if self.state.section == ContainerSection::Containers
            && self.state.selected.is_none()
            && inspect_first.is_none()
            && let Some(first) = self.state.containers.first()
        {
            inspect_first = Some(first.id.clone());
        }
        if let Some(id) = inspect_first {
            self.select_container_item(id);
        } else if let Some(id) = self.state.selected.clone() {
            let request_id = ContainerRequestId::new();
            self.send_tracked(
                request_id,
                ContainerCommand::InspectContainer {
                    tab_id: self.tab_id,
                    request_id,
                    container: id,
                },
            );
        }
    }

    /// 应用容器详情响应。
    pub(crate) fn apply_container_inspected(&mut self, details: transom_core::ContainerDetails) {
        self.state.selected = Some(details.id.clone());
        self.state.details = Some(details);
    }

    /// 应用镜像列表响应：修正选中项并自动 inspect 第一项。
    pub(crate) fn apply_images_listed(&mut self, images: Vec<transom_core::ContainerImageSummary>) {
        self.state.images = images;
        self.state.busy = false;
        let valid = self
            .state
            .selected
            .as_deref()
            .is_some_and(|id| self.state.images.iter().any(|item| item.id == id));
        if !valid {
            self.state.selected = self.state.images.first().map(|item| item.id.clone());
            self.state.image_details = None;
        }
        if let Some(id) = self.state.selected.clone() {
            let request_id = ContainerRequestId::new();
            self.send_tracked(
                request_id,
                ContainerCommand::InspectImage {
                    tab_id: self.tab_id,
                    request_id,
                    image: id,
                },
            );
        }
    }

    /// 应用镜像详情响应。
    pub(crate) fn apply_image_inspected(&mut self, details: transom_core::ContainerImageDetails) {
        self.state.selected = Some(details.id.clone());
        self.state.image_details = Some(details);
    }

    /// 应用数据卷列表响应：修正选中项并自动 inspect 第一项。
    pub(crate) fn apply_volumes_listed(
        &mut self,
        volumes: Vec<transom_core::ContainerVolumeSummary>,
    ) {
        self.state.volumes = volumes;
        self.state.busy = false;
        let valid = self
            .state
            .selected
            .as_deref()
            .is_some_and(|name| self.state.volumes.iter().any(|item| item.name == name));
        if !valid {
            self.state.selected = self.state.volumes.first().map(|item| item.name.clone());
            self.state.volume_details = None;
        }
        if let Some(name) = self.state.selected.clone() {
            let request_id = ContainerRequestId::new();
            self.send_tracked(
                request_id,
                ContainerCommand::InspectVolume {
                    tab_id: self.tab_id,
                    request_id,
                    name,
                },
            );
        }
    }

    /// 应用数据卷详情响应。
    pub(crate) fn apply_volume_inspected(&mut self, details: transom_core::ContainerVolumeDetails) {
        self.state.selected = Some(details.summary.name.clone());
        self.state.volume_details = Some(details);
    }

    /// 应用网络列表响应：修正选中项并自动 inspect 第一项。
    pub(crate) fn apply_networks_listed(
        &mut self,
        networks: Vec<transom_core::ContainerNetworkSummary>,
    ) {
        self.state.networks = networks;
        self.state.busy = false;
        let valid = self
            .state
            .selected
            .as_deref()
            .is_some_and(|id| self.state.networks.iter().any(|item| item.id == id));
        if !valid {
            self.state.selected = self.state.networks.first().map(|item| item.id.clone());
            self.state.network_details = None;
        }
        if let Some(id) = self.state.selected.clone() {
            let request_id = ContainerRequestId::new();
            self.send_tracked(
                request_id,
                ContainerCommand::InspectNetwork {
                    tab_id: self.tab_id,
                    request_id,
                    network: id,
                },
            );
        }
    }

    /// 应用网络详情响应。
    pub(crate) fn apply_network_inspected(
        &mut self,
        details: transom_core::ContainerNetworkDetails,
    ) {
        self.state.selected = Some(details.summary.id.clone());
        self.state.network_details = Some(details);
    }

    /// 应用 Compose 项目列表响应。
    pub(crate) fn apply_compose_projects_listed(
        &mut self,
        projects: Vec<transom_core::ComposeProject>,
    ) {
        self.state.compose = projects;
        self.state.busy = false;
    }

    /// 应用容器进程列表响应。
    pub(crate) fn apply_processes_listed(
        &mut self,
        processes: Vec<transom_core::ContainerProcess>,
    ) {
        self.state.stats_processes_request = None;
        self.state.stats_processes_loading = false;
        self.state.stats_processes = processes;
        self.state.stats_processes_error = None;
    }
}

impl ContainerWorkspace {
    /// 应用一条动作结果：收尾忙碌/确认/弹窗状态，返回需要回读的预览编辑器。
    ///
    /// 设计原理（Why）：动作结果涉及确认框、重命名、Pod/Quadlet、构建、预览保存
    /// 等多处状态，集中在一个实体方法里比在外壳闭包里堆分支更容易核对。
    pub(crate) fn apply_action_finished(
        &mut self,
        request_id: ContainerRequestId,
        error: Option<&AppError>,
    ) -> Option<Entity<gpui_kit::component::input::TextareaState>> {
        self.state.busy = false;
        self.state.confirm = None;
        self.state.rename_container = None;
        self.state.notice = None;
        if self.state.pods_action_request == Some(request_id) {
            self.state.pods_action_request = None;
            self.state.pods_loading = false;
            if let Some(err) = error {
                self.state.pods_error = Some(localized_error(err));
            }
        }
        if self.state.quadlets_action_request == Some(request_id) {
            self.state.quadlets_action_request = None;
            self.state.quadlets_loading = false;
            if let Some(err) = error {
                self.state.quadlets_error = Some(localized_error(err));
            }
        }
        if let Some(build) = self.state.build_dialog.as_mut()
            && build.request_id == Some(request_id)
        {
            build.building = false;
            if let Some(err) = error {
                build.error = Some(localized_error(err));
            } else {
                build.success = true;
            }
        }
        if self.state.files.preview_saving {
            self.state.files.preview_saving = false;
            if error.is_none() {
                return self.state.files.preview_editor.take();
            }
        }
        None
    }

    /// 用编辑器当前文本刷新预览内容。
    pub(crate) fn apply_preview_saved(&mut self, content: String) {
        if let Some((path, _, _)) = self.state.files.preview.as_ref() {
            let path = path.clone();
            self.state.files.preview = Some((path, content, false));
        }
    }

    /// 应用一次传输结果：返回是否需要刷新镜像列表或重新加载目录。
    pub(crate) fn apply_transfer_finished(
        &mut self,
        error: Option<&AppError>,
    ) -> (bool, Option<String>) {
        self.state.notice = None;
        if error.is_some() {
            return (false, None);
        }
        if self.state.section == ContainerSection::Images {
            (true, None)
        } else {
            (false, Some(self.state.files.path.clone()))
        }
    }

    /// 应用一次非传输失败：按请求 id 清理所有可能挂在该请求上的状态。
    pub(crate) fn apply_failed(&mut self, request_id: ContainerRequestId, err_desc: &str) {
        self.state.busy = false;
        self.state.files.loading = false;
        self.state.notice = None;
        if self.state.logs_request == Some(request_id) {
            self.state.logs_request = None;
            self.state.logs_follow = false;
        }
        if self.state.stats_request == Some(request_id) {
            self.state.stats_request = None;
            self.state.stats_error = Some(err_desc.to_string());
        }
        if self.state.stats_processes_request == Some(request_id) {
            self.state.stats_processes_request = None;
            self.state.stats_processes_loading = false;
            let friendly_err = if err_desc.to_lowercase().contains("is not running") {
                rust_i18n::t!("container.stats.container_stopped").to_string()
            } else {
                err_desc.to_string()
            };
            self.state.stats_processes_error = Some(friendly_err);
        }
        if self.state.files.request == Some(request_id) {
            self.state.files.request = None;
            self.state.files.loading = false;
        }
        if self.state.pods_request == Some(request_id) {
            self.state.pods_request = None;
            self.state.pods_loading = false;
            self.state.pods_error = Some(err_desc.to_string());
        }
        if self.state.pods_action_request == Some(request_id) {
            self.state.pods_action_request = None;
            self.state.pods_loading = false;
            self.state.pods_error = Some(err_desc.to_string());
        }
        if self.state.quadlets_request == Some(request_id) {
            self.state.quadlets_request = None;
            self.state.quadlets_loading = false;
            self.state.quadlets_error = Some(err_desc.to_string());
        }
        if self.state.quadlets_action_request == Some(request_id) {
            self.state.quadlets_action_request = None;
            self.state.quadlets_loading = false;
            self.state.quadlets_error = Some(err_desc.to_string());
        }
        if self.state.changes_request == Some(request_id) {
            self.state.changes_request = None;
            self.state.changes_loading = false;
        }
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            if hub.search_request == Some(request_id) {
                hub.searching = false;
                hub.search_request = None;
                hub.search_error = Some(err_desc.to_string());
            }
            if hub.tags_request == Some(request_id) {
                hub.tags_loading = false;
                hub.tags_request = None;
                hub.tags_error = Some(err_desc.to_string());
            }
        }
        if self
            .state
            .terminal_request
            .as_ref()
            .is_some_and(|(pending, ..)| *pending == request_id)
        {
            self.state.terminal_request = None;
        }
    }

    /// 应用全局磁盘占用响应。
    pub(crate) fn apply_system_df_loaded(
        &mut self,
        request_id: ContainerRequestId,
        usage: transom_core::ContainerSystemDiskUsage,
    ) {
        if self.state.system_df_request == Some(request_id) {
            self.state.system_df_request = None;
        }
        self.state.system_df_loading = false;
        self.state.system_df = Some(usage);
    }

    /// 收尾系统清理状态。
    pub(crate) fn apply_system_pruned(&mut self) {
        self.state.system_prune_running = false;
    }

    /// 应用文件差异对比响应。
    pub(crate) fn apply_diffed(
        &mut self,
        request_id: ContainerRequestId,
        container: String,
        changes: Vec<transom_core::ContainerFileSystemChange>,
    ) {
        if self.state.changes_request == Some(request_id) {
            self.state.changes_request = None;
        }
        self.state.changes_loading = false;
        self.state.changes = Some((container, changes));
    }
}

impl ContainerWorkspace {
    /// 追加一段日志；请求已被替换时丢弃。
    pub(crate) fn apply_logs_chunk(&mut self, request_id: ContainerRequestId, bytes: &[u8]) {
        if self.state.logs_request == Some(request_id) {
            self.state.append_logs(bytes);
        }
    }

    /// 收尾日志流。
    pub(crate) fn apply_logs_closed(&mut self, request_id: ContainerRequestId) {
        if self.state.logs_request == Some(request_id) {
            self.state.logs_request = None;
            self.state.logs_follow = false;
        }
    }

    /// 应用一帧统计：维护高负载告警与历史采样。
    pub(crate) fn apply_stats_updated(
        &mut self,
        request_id: ContainerRequestId,
        mut stats: transom_core::ContainerStats,
    ) {
        if self.state.stats_request != Some(request_id) {
            return;
        }
        if stats.host_load.is_none()
            && let Some(prev) = &self.state.stats
            && prev.host_load.is_some()
        {
            stats.host_load = prev.host_load;
        }
        self.state.stats_high_cpu = if stats.cpu_percent >= 90.0 {
            self.state.stats_high_cpu.saturating_add(1)
        } else {
            0
        };
        self.state.stats_high_memory = if stats.memory_percent >= 90.0 {
            self.state.stats_high_memory.saturating_add(1)
        } else {
            0
        };
        let high_cpu = self.state.stats_high_cpu >= 3;
        let high_memory = self.state.stats_high_memory >= 3;
        let alert_due = self
            .state
            .stats_alert_at
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(300));
        if (high_cpu || high_memory) && alert_due {
            self.state.stats_alert_at = Some(std::time::Instant::now());
            self.state.notice = Some(if high_cpu {
                rust_i18n::t!("container.stats.alert_cpu").to_string()
            } else {
                rust_i18n::t!("container.stats.alert_memory").to_string()
            });
        }
        self.state.stats = Some(stats.clone());
        let due = self
            .state
            .stats_last_append
            .is_none_or(|last| last.elapsed() >= self.state.stats_interval.interval());
        if due {
            self.state.stats_last_append = Some(std::time::Instant::now());
            self.state.stats_history.push_back(stats);
            while self.state.stats_history.len() > STATS_HISTORY_LIMIT {
                self.state.stats_history.pop_front();
            }
        }
        self.state.stats_error = None;
    }

    /// 记录统计暂不可用的原因。
    pub(crate) fn apply_stats_unavailable(&mut self, request_id: ContainerRequestId, error: &str) {
        if self.state.stats_request == Some(request_id) {
            self.state.stats_error =
                Some(rust_i18n::t!("container.stats.reconnecting", error = error).to_string());
        }
    }

    /// 应用目录列表响应。
    pub(crate) fn apply_dir_listed(
        &mut self,
        request_id: ContainerRequestId,
        path: String,
        entries: Vec<transom_core::ContainerFileEntry>,
    ) {
        if self.state.files.request != Some(request_id) {
            return;
        }
        log::info!("容器目录已加载，路径: {path}，条目: {}", entries.len());
        self.state.files.request = None;
        self.state.files.path = path;
        self.state.files.entries = entries;
        self.state.files.loading = false;
        self.state
            .files
            .scroll
            .set_offset(gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(0.0)));
    }

    /// 应用文件预览响应。
    pub(crate) fn apply_file_preview(
        &mut self,
        path: String,
        content: String,
        truncated: bool,
        binary: bool,
    ) {
        self.state.files.preview = Some((path, content, truncated));
        self.state.files.preview_binary = binary;
        if binary {
            self.state.files.preview_editor = None;
        }
    }

    /// 记录传输进度提示。
    pub(crate) fn apply_transfer_progress(&mut self, transferred: u64, total: Option<u64>) {
        self.state.notice = Some(match total {
            Some(total) if total > 0 => format!(
                "{} / {}",
                crate::format::format_size(transferred),
                crate::format::format_size(total)
            ),
            _ => crate::format::format_size(transferred),
        });
    }

    /// 应用构建进度：更新提示并把日志追加到构建弹窗。
    pub(crate) fn apply_build_progress(&mut self, request_id: ContainerRequestId, line: &str) {
        self.state.notice = line.lines().last().map(str::to_string);
        if let Some(build) = self
            .state
            .build_dialog
            .as_mut()
            .filter(|build| build.request_id == Some(request_id))
        {
            build.logs.extend(line.lines().map(str::to_string));
            if build.logs.len() > crate::container::types::BUILD_LOG_LIMIT {
                let excess = build.logs.len() - crate::container::types::BUILD_LOG_LIMIT;
                build.logs.drain(..excess);
            }
        }
    }
}

impl ContainerWorkspace {
    /// 应用分层下载进度。
    pub(crate) fn apply_pull_progress(
        &mut self,
        task_id: &str,
        status: String,
        layers: Vec<transom_core::ImageLayerProgress>,
    ) {
        self.state.pull_manager.on_progress(task_id, status, layers);
    }

    /// 应用下载任务结束：返回任务引用与队列中下一个待启动命令。
    ///
    /// 设计原理（Why）：`on_finished` 会推进队列并返回下一条命令，外壳只负责
    /// 弹提示与投递；把队列状态留在实体里，避免外壳闭包操作下载管理器内部。
    pub(crate) fn apply_pull_finished(
        &mut self,
        task_id: &str,
        error_msg: Option<String>,
    ) -> Option<PullFinishedOutcome> {
        let task = self.state.pull_manager.get(task_id)?;
        let reference = task.reference.clone();
        let next = self.state.pull_manager.on_finished(task_id, error_msg);
        Some((reference, next))
    }
}
