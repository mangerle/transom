//! SFTP 传输队列调度、入队与批量控制。
//!
//! 设计原理（Why）：
//! - 实现初衷：负责 SFTP 传输队列的并发调度、任务分发与整列批量控制（暂停全部、恢复全部、取消全部）；
//! - 核心优势：
//!   1. 集中管理并发调度循环 `process_sftp_queue`；
//!   2. 规范化整批操作日志与命令发送顺序。

mod enqueue;
mod task_control;

use self::enqueue::upload_tasks;
use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    AppCommand, SftpOperation, TabId, TransferDirection, TransferFilter, TransferId, TransferKind,
    TransferMode, TransferStatus, TransferTask,
};

impl AppShell {
    pub(crate) fn sftp_queue_upload(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let paths = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| panel.selected_local_paths())
            .unwrap_or_default();
        if let Err(message) = self.enqueue_uploads(tab_id, &paths, cx) {
            self.set_sftp_panel_error(tab_id, message, cx);
        }
    }

    /// 把一组本地路径入队为上传任务并启动调度。
    pub(crate) fn enqueue_uploads(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        if paths.is_empty() {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        }
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let tasks = upload_tasks(paths, &panel.path, panel.resume_enabled, verify)?;
        let count = tasks.len();
        log::info!(
            "上传任务入队，窗格: {tab_id}，新增 {count} 项，目标目录: {}，队列现有 {} 项，校验: {verify}",
            panel.path,
            panel.queue.len() + count
        );
        panel.queue.extend(tasks);
        panel.status = Some(rust_i18n::t!("sftp.upload_queued", count = count).to_string());
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
        Ok(count)
    }

    fn set_sftp_panel_error(&mut self, tab_id: TabId, message: String, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            panel.error = Some(message);
            cx.notify();
        }
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
        let mut invalid_name: Option<String> = None;
        for path in paths {
            let Some(remote_entry) = panel.entries.iter().find(|entry| entry.path == path) else {
                continue;
            };
            let local_path =
                match transom_core::safe_local_join(&panel.local_path, &remote_entry.name) {
                    Ok(path) => path,
                    Err(error) => {
                        invalid_name = Some(localized_error(&error));
                        break;
                    }
                };
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
        if let Some(message) = invalid_name {
            panel.error = Some(message);
            cx.notify();
            return;
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
        let tab_conn_id = tab.connection_id;
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
            let target_conn = task
                .target_connection_id
                .or(task.source_connection_id)
                .or(tab_conn_id)
                .or_else(|| panel.side_remotes.first().map(|s| s.connection_id));

            let Some(conn_id) = target_conn else {
                panel.queue[index].status =
                    TransferStatus::Failed("未找到可用的远端连接".to_string());
                continue;
            };

            let Some(op) = sftp_operation_for_task(&task, &filter) else {
                panel.queue[index].status =
                    TransferStatus::Failed("不支持的传输任务类型组合".to_string());
                continue;
            };
            panel.queue[index].status = TransferStatus::Transferring;
            panel.active_transfers.insert(task.id);
            commands.push((task.id, conn_id, op));
        }

        if commands.is_empty() {
            panel.recompute_progress();
            cx.notify();
            return;
        }
        panel.error = None;
        panel.recompute_progress();
        for (op_id, connection_id, op) in commands {
            self.send_command(AppCommand::Sftp {
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

    /// 暂停全部活动与排队任务；队列收起时由状态栏调用。
    pub(crate) fn sftp_pause_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        log::debug!("暂停整列传输任务，窗格: {tab_id}");
        let mut active_ids: Vec<TransferId> = Vec::new();
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            active_ids = panel.active_transfers.iter().copied().collect();
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
            panel.active_transfers.clear();
            panel.recompute_progress();
            if paused > 0 {
                panel.status = Some(format!("已暂停 {paused} 个传输任务"));
            }
            cx.notify();
        }
        for op_id in active_ids {
            self.send_command(AppCommand::SftpCancel { tab_id, op_id });
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
        log::debug!("取消整列传输任务，窗格: {tab_id}");
        let mut active_ids: Vec<TransferId> = Vec::new();
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            active_ids = panel.active_transfers.iter().copied().collect();
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
            panel.active_transfers.clear();
            panel.recompute_progress();
            if cancelled > 0 {
                panel.status = Some(format!("已取消 {cancelled} 个传输任务"));
            }
            cx.notify();
        }
        for op_id in active_ids {
            self.send_command(AppCommand::SftpCancel { tab_id, op_id });
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
        (_, TransferDirection::RemoteCopy) => {
            let source_conn = task.source_connection_id?;
            let target_conn = task.target_connection_id?;
            let target_path = task.target_remote_path.clone()?;
            SftpOperation::RemoteTransfer {
                source_connection_id: source_conn,
                source_path: task.remote_path.clone(),
                target_connection_id: target_conn,
                target_path,
                is_dir: task.kind == TransferKind::Directory,
                filter: filter.clone(),
                preserve_attributes: task.preserve_attributes,
            }
        }
        _ => return None,
    };
    Some(op)
}
