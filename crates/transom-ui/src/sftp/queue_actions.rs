//! SFTP 传输队列调度、上传下载入队、暂停恢复取消与置顶操作方法。

use super::join_remote;
use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{
    AppCommand, SftpOperation, TabId, TransferDirection, TransferFilter, TransferId, TransferKind,
    TransferMode, TransferStatus, TransferTask,
};

impl AppShell {
    pub(crate) fn sftp_queue_upload(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        let paths = panel.selected_local_paths();
        if paths.is_empty() {
            panel.error = Some(rust_i18n::t!("sftp.upload_file").to_string());
            cx.notify();
            return;
        }
        let mode = if panel.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut tasks = Vec::with_capacity(paths.len());
        for local_path in paths {
            let Some(file_name) = local_path.file_name().and_then(|name| name.to_str()) else {
                panel.error = Some(rust_i18n::t!("sftp.upload_file").to_string());
                cx.notify();
                return;
            };
            let remote_path = join_remote(&panel.path, file_name);
            let is_directory = std::fs::symlink_metadata(&local_path)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            let size = if is_directory {
                None
            } else {
                std::fs::metadata(&local_path).map(|meta| meta.len()).ok()
            };
            let mut task = if is_directory {
                TransferTask::new_directory(
                    TransferDirection::Upload,
                    local_path,
                    remote_path,
                    size,
                    mode,
                )
            } else {
                TransferTask::new(
                    TransferDirection::Upload,
                    local_path,
                    remote_path,
                    size,
                    mode,
                )
            };
            task.verify = verify;
            tasks.push(task);
        }
        let count = tasks.len();
        panel.queue.extend(tasks);
        panel.status = Some(format!("已加入 {count} 个上传任务"));
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn sftp_queue_download(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        let paths = panel.selected_remote_paths();
        if paths.is_empty() {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let mode = if panel.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut tasks = Vec::with_capacity(paths.len());
        for path in paths {
            let Some(remote_entry) = panel.entries.iter().find(|entry| entry.path == path) else {
                continue;
            };
            let local_path = panel.local_path.join(&remote_entry.name);
            let mut task = if remote_entry.is_dir {
                TransferTask::new_directory(
                    TransferDirection::Download,
                    local_path,
                    remote_entry.path.clone(),
                    None,
                    mode,
                )
            } else {
                TransferTask::new(
                    TransferDirection::Download,
                    local_path,
                    remote_entry.path.clone(),
                    Some(remote_entry.size),
                    mode,
                )
            };
            task.verify = verify;
            tasks.push(task);
        }
        if tasks.is_empty() {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let count = tasks.len();
        panel.queue.extend(tasks);
        panel.status = Some(format!("已加入 {count} 个下载任务"));
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }

    /// 按并发上限启动待处理任务；任一任务完成或取消后再次调用即可补位。
    pub(crate) fn process_sftp_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let filter = TransferFilter::from_text(&self.settings.sftp.transfer_filter);
        let max_concurrent = self.settings.sftp.max_concurrent_transfers.clamp(1, 8);
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };

        let mut commands = Vec::new();
        while panel.active_transfers.len() < max_concurrent {
            let Some(index) = panel
                .queue
                .iter()
                .position(|task| task.status == TransferStatus::Pending)
            else {
                break;
            };
            let task = panel.queue[index].clone();
            let Some(op) = sftp_operation_for_task(&task, &filter) else {
                panel.queue[index].status =
                    TransferStatus::Failed("不支持的传输任务类型组合".to_string());
                continue;
            };
            panel.queue[index].status = TransferStatus::Transferring;
            panel.active_transfers.insert(task.id);
            commands.push((task.id, op));
        }

        if commands.is_empty() {
            panel.recompute_progress();
            cx.notify();
            return;
        }
        panel.error = None;
        panel.recompute_progress();
        for (op_id, op) in commands {
            let _ = self.services.command_tx.send(AppCommand::Sftp {
                tab_id,
                connection_id,
                op_id,
                op,
            });
        }
        cx.notify();
    }

    pub(crate) fn sftp_queue_clear_done(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            panel.queue.retain(|task| {
                task.status != TransferStatus::Completed && task.status != TransferStatus::Cancelled
            });
            cx.notify();
        }
    }

    /// 循环切换并发传输数；增加时立即补位启动待处理任务。
    pub(crate) fn sftp_cycle_concurrency(&mut self, cx: &mut Context<Self>) {
        let current = self.settings.sftp.max_concurrent_transfers.clamp(1, 4);
        let next = if current >= 4 { 1 } else { current + 1 };
        self.settings.sftp.max_concurrent_transfers = next;
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        }
        self.status = Some(format!("并发传输数: {next}"));
        if let Some(tab_id) = self.active_tab {
            self.process_sftp_queue(tab_id, cx);
        }
        cx.notify();
    }

    /// 一键重试传输队列中全部处于失败状态（`TransferStatus::Failed`）的任务。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：遭遇网络波动或会话临时中断时，队列内可能同时有多个文件传输失败，
    ///   提供批量一键重试避免用户逐行手动点击重发。
    /// - 核心优势：将所有失败项统一重置为 `Pending` 并根据当前是否开启断点续传或已传输字节数启用 `Resume` 模式，
    ///   随后立即唤醒队列调度器按序处理。
    /// - 代价与局限：批量重试按队列原有顺序依次执行，不会改变队列内任务的相对先后次序。
    pub(crate) fn sftp_retry_all_failed(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut retried_count: usize = 0;
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            let resume_enabled = panel.resume_enabled;
            for task in &mut panel.queue {
                if matches!(task.status, TransferStatus::Failed(_)) {
                    task.status = TransferStatus::Pending;
                    if resume_enabled || task.transferred > 0 {
                        task.mode = TransferMode::Resume;
                    }
                    retried_count = retried_count.saturating_add(1);
                }
            }
            if retried_count > 0 {
                panel.error = None;
                panel.status = Some(format!("已重新入队 {retried_count} 个失败传输任务"));
                cx.notify();
            }
        }
        if retried_count > 0 {
            self.process_sftp_queue(tab_id, cx);
        }
    }

    pub(crate) fn sftp_pause_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id)
                && task.status == TransferStatus::Transferring
            {
                let _ = self
                    .services
                    .command_tx
                    .send(AppCommand::SftpCancel { tab_id, op_id });
                task.status = TransferStatus::Paused;
            }
            panel.active_transfers.remove(&op_id);
            panel.recompute_progress();
            if panel.active_transfers.is_empty() {
                panel.status = Some("传输已暂停".to_string());
            }
            cx.notify();
        }
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn sftp_resume_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id)
                && (task.status == TransferStatus::Paused
                    || matches!(task.status, TransferStatus::Failed(_)))
            {
                task.status = TransferStatus::Pending;
                task.mode = TransferMode::Resume;
            }
            cx.notify();
        }
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn sftp_cancel_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            let mut should_cancel = false;
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                if task.status == TransferStatus::Transferring {
                    should_cancel = true;
                }
                task.status = TransferStatus::Cancelled;
            }
            if should_cancel {
                let _ = self
                    .services
                    .command_tx
                    .send(AppCommand::SftpCancel { tab_id, op_id });
            }
            panel.active_transfers.remove(&op_id);
            panel.recompute_progress();
            if panel.active_transfers.is_empty() {
                panel.status = Some("已取消传输".to_string());
            }
            cx.notify();
        }
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn sftp_remove_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            panel.active_transfers.remove(&op_id);
            panel.queue.retain(|task| task.id != op_id);
            panel.recompute_progress();
            cx.notify();
        }
    }

    pub(crate) fn sftp_prioritize_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            if panel.active_transfers.contains(&op_id)
                && let Some(active_task) = panel.queue.iter_mut().find(|t| t.id == op_id)
                && active_task.status == TransferStatus::Transferring
            {
                let _ = self
                    .services
                    .command_tx
                    .send(AppCommand::SftpCancel { tab_id, op_id });
                active_task.status = TransferStatus::Pending;
                active_task.mode = TransferMode::Resume;
                panel.active_transfers.remove(&op_id);
            }

            if let Some(pos) = panel.queue.iter().position(|t| t.id == op_id) {
                let mut task = panel.queue.remove(pos);
                task.status = TransferStatus::Pending;
                if task.transferred > 0 {
                    task.mode = TransferMode::Resume;
                }
                panel.queue.insert(0, task);
                panel.status = Some("已将传输任务置顶并优先开始传输".to_string());
            }
            panel.recompute_progress();
            cx.notify();
        }
        self.process_sftp_queue(tab_id, cx);
    }

    /// 暂停全部活动与排队任务；队列收起时由状态栏调用。
    pub(crate) fn sftp_pause_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            let active_ids = panel.active_transfers.iter().copied().collect::<Vec<_>>();
            let mut paused = 0u32;
            for task in &mut panel.queue {
                match task.status {
                    TransferStatus::Transferring | TransferStatus::Pending => {
                        task.status = TransferStatus::Paused;
                        paused = paused.saturating_add(1);
                    }
                    _ => {}
                }
            }
            for op_id in active_ids {
                let _ = self
                    .services
                    .command_tx
                    .send(AppCommand::SftpCancel { tab_id, op_id });
            }
            panel.active_transfers.clear();
            panel.recompute_progress();
            if paused > 0 {
                panel.status = Some(format!("已暂停 {paused} 个传输任务"));
            }
            cx.notify();
        }
    }

    /// 继续全部暂停任务。
    pub(crate) fn sftp_resume_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut resumed = 0u32;
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            for task in &mut panel.queue {
                if task.status == TransferStatus::Paused {
                    task.status = TransferStatus::Pending;
                    task.mode = TransferMode::Resume;
                    resumed = resumed.saturating_add(1);
                }
            }
            if resumed > 0 {
                panel.status = Some(format!("已继续 {resumed} 个传输任务"));
            }
            cx.notify();
        }
        if resumed > 0 {
            self.process_sftp_queue(tab_id, cx);
        }
    }

    /// 取消全部活动、排队与暂停任务。
    pub(crate) fn sftp_cancel_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            let active_ids = panel.active_transfers.iter().copied().collect::<Vec<_>>();
            let mut cancelled = 0u32;
            for task in &mut panel.queue {
                if matches!(
                    task.status,
                    TransferStatus::Transferring | TransferStatus::Pending | TransferStatus::Paused
                ) {
                    task.status = TransferStatus::Cancelled;
                    cancelled = cancelled.saturating_add(1);
                }
            }
            for op_id in active_ids {
                let _ = self
                    .services
                    .command_tx
                    .send(AppCommand::SftpCancel { tab_id, op_id });
            }
            panel.active_transfers.clear();
            panel.recompute_progress();
            if cancelled > 0 {
                panel.status = Some(format!("已取消 {cancelled} 个传输任务"));
            }
            cx.notify();
        }
    }
}

fn sftp_operation_for_task(task: &TransferTask, filter: &TransferFilter) -> Option<SftpOperation> {
    let resume = task.mode != TransferMode::Overwrite;
    let op = match (task.kind, task.direction) {
        (TransferKind::Directory, TransferDirection::Upload) => SftpOperation::UploadDir {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            resume,
            filter: filter.clone(),
            verify: task.verify,
        },
        (TransferKind::Directory, TransferDirection::Download) => SftpOperation::DownloadDir {
            remote_path: task.remote_path.clone(),
            local_path: task.local_path.clone(),
            resume,
            filter: filter.clone(),
            verify: task.verify,
        },
        (TransferKind::File, TransferDirection::Upload) => SftpOperation::Upload {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            resume,
            verify: task.verify,
        },
        (TransferKind::File, TransferDirection::Download) => SftpOperation::Download {
            remote_path: task.remote_path.clone(),
            local_path: task.local_path.clone(),
            resume,
            verify: task.verify,
        },
        (TransferKind::Sync, TransferDirection::Sync) => SftpOperation::SyncDirs {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            items: task.sync_items.clone(),
            verify: task.verify,
        },
        _ => return None,
    };
    Some(op)
}
