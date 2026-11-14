//! 日志、统计与文件事件写回实体的具名方法。

use super::PullFinishedOutcome;
use super::super::super::*;

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
