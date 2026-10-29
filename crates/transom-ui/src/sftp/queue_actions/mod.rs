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
use crate::sftp::{TransferKind, TransferMode, TransferStatus, TransferTask};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    ConnectionId, SftpCommand, SftpOperation, TabId, TransferDirection, TransferFilter, TransferId,
};

impl AppShell {
    pub(crate) fn sftp_queue_upload(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let paths = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
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
        let verify_mode = self.settings.sftp.verify_mode;
        let algorithm = self.settings.sftp.checksum_algorithm;
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let Some(target_connection_id) = tab.connection_id else {
            return Err(rust_i18n::t!("sftp.queue_actions.no_remote_upload").to_string());
        };
        let Some(panel) = tab.sftp_mut() else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let tasks = upload_tasks(
            paths,
            &panel.remote.path,
            panel.queue.resume_enabled,
            verify,
            verify_mode,
            algorithm,
            target_connection_id,
        )?;
        let count = tasks.len();
        log::info!(
            "上传任务入队，窗格: {tab_id}，新增 {count} 项，目标目录: {}，队列现有 {} 项，校验: {verify}",
            panel.remote.path,
            panel.queue.tasks.len() + count
        );
        panel.queue.tasks.extend(tasks);
        panel.set_status(rust_i18n::t!("sftp.upload_queued", count = count).to_string());
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
        Ok(count)
    }

    fn set_sftp_panel_error(&mut self, tab_id: TabId, message: String, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.set_error(message);
            cx.notify();
        }
    }

    pub(crate) fn sftp_queue_download(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let source_connection_id = tab.connection_id;
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        let Some(source_connection_id) = source_connection_id else {
            panel.set_error(rust_i18n::t!("sftp.queue_actions.no_remote_download").to_string());
            cx.notify();
            return;
        };
        let paths = panel.selected_remote_paths();
        if paths.is_empty() {
            panel.set_error(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let mode = if panel.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut tasks = Vec::with_capacity(paths.len());
        let mut invalid_name: Option<String> = None;
        for path in paths {
            let Some(remote_entry) = panel.remote.entries.iter().find(|entry| entry.path == path)
            else {
                continue;
            };
            let local_path =
                match transom_core::safe_local_join(&panel.local.path, &remote_entry.name) {
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
            task.source_connection_id = Some(source_connection_id);
            task.verify = verify;
            task.verify_mode = self.settings.sftp.verify_mode;
            task.checksum_algorithm = self.settings.sftp.checksum_algorithm;
            tasks.push(task);
        }
        if let Some(message) = invalid_name {
            panel.set_error(message);
            cx.notify();
            return;
        }
        if tasks.is_empty() {
            panel.set_error(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let count = tasks.len();
        panel.queue.tasks.extend(tasks);
        panel.set_status(
            rust_i18n::t!("sftp.queue_actions.downloads_queued", count = count).to_string(),
        );
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }

    /// 按并发上限启动待处理任务；任一任务完成或取消后再次调用即可补位。
    pub(crate) fn process_sftp_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let filter = TransferFilter::from_text(&self.settings.sftp.transfer_filter);
        let max_concurrent = self.settings.sftp.max_concurrent_transfers.clamp(1, 8);
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp_mut() else {
            return;
        };

        let mut commands = Vec::new();
        while panel.queue.active_transfers.len() < max_concurrent {
            let Some(index) = panel
                .queue
                .tasks
                .iter()
                .position(|task| task.status == TransferStatus::Pending)
            else {
                break;
            };
            let task = panel.queue.tasks[index].clone();
            let Some(conn_id) = task_connection_id(&task) else {
                panel.queue.tasks[index].status = TransferStatus::Failed(
                    rust_i18n::t!("sftp.queue_actions.missing_connection").to_string(),
                );
                continue;
            };

            let Some(op) = sftp_operation_for_task(&task, &filter) else {
                panel.queue.tasks[index].status = TransferStatus::Failed(
                    rust_i18n::t!("sftp.queue_actions.unsupported_task").to_string(),
                );
                continue;
            };
            panel.queue.tasks[index].status = TransferStatus::Transferring;
            panel.queue.active_transfers.insert(task.id);
            commands.push((task.id, conn_id, op));
        }

        if commands.is_empty() {
            panel.recompute_progress();
            cx.notify();
            return;
        }
        panel.clear_error();
        panel.recompute_progress();
        for (op_id, connection_id, op) in commands {
            self.send_command(SftpCommand::Sftp {
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
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.queue.tasks.retain(|task| {
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
            self.set_error(localized_error(&error));
        }
        self.status =
            Some(rust_i18n::t!("sftp.queue_actions.concurrency_set", count = next).to_string());
        if let Some(tab_id) = self.workspace.active_tab {
            self.process_sftp_queue(tab_id, cx);
        }
        cx.notify();
    }

    /// 一键重试传输队列中全部处于失败状态（`TransferStatus::Failed`）的任务。
    pub(crate) fn sftp_retry_all_failed(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut retried_count: usize = 0;
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            let resume_enabled = panel.queue.resume_enabled;
            for task in &mut panel.queue.tasks {
                if let TransferStatus::Failed(ref reason) = task.status {
                    let is_verify_failure = reason.contains("校验") || reason.contains("verify");
                    task.status = TransferStatus::Pending;
                    if is_verify_failure {
                        task.mode = TransferMode::Overwrite;
                        task.transferred = 0;
                    } else if resume_enabled || task.transferred > 0 {
                        task.mode = TransferMode::Resume;
                    }
                    retried_count = retried_count.saturating_add(1);
                }
            }
            if retried_count > 0 {
                panel.clear_error();
                panel.set_status(
                    rust_i18n::t!("sftp.queue_actions.retried", count = retried_count).to_string(),
                );
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
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            active_ids = panel.queue.active_transfers.iter().copied().collect();
            let mut paused = 0u32;
            for task in &mut panel.queue.tasks {
                match task.status {
                    TransferStatus::Transferring | TransferStatus::Pending => {
                        task.status = TransferStatus::Paused;
                        paused = paused.saturating_add(1);
                    }
                    _ => {}
                }
            }
            panel.queue.active_transfers.clear();
            panel.recompute_progress();
            if paused > 0 {
                panel.set_status(
                    rust_i18n::t!("sftp.queue_actions.paused", count = paused).to_string(),
                );
            }
            cx.notify();
        }
        for op_id in active_ids {
            self.send_command(SftpCommand::SftpCancel { tab_id, op_id });
        }
    }

    /// 继续全部暂停任务。
    pub(crate) fn sftp_resume_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut resumed = 0u32;
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            for task in &mut panel.queue.tasks {
                if task.status == TransferStatus::Paused {
                    task.status = TransferStatus::Pending;
                    task.mode = TransferMode::Resume;
                    resumed = resumed.saturating_add(1);
                }
            }
            if resumed > 0 {
                panel.set_status(
                    rust_i18n::t!("sftp.queue_actions.resumed", count = resumed).to_string(),
                );
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
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            active_ids = panel.queue.active_transfers.iter().copied().collect();
            let mut cancelled = 0u32;
            for task in &mut panel.queue.tasks {
                if matches!(
                    task.status,
                    TransferStatus::Transferring | TransferStatus::Pending | TransferStatus::Paused
                ) {
                    task.status = TransferStatus::Cancelled;
                    cancelled = cancelled.saturating_add(1);
                }
            }
            panel.queue.active_transfers.clear();
            panel.recompute_progress();
            if cancelled > 0 {
                panel.set_status(
                    rust_i18n::t!("sftp.queue_actions.cancelled", count = cancelled).to_string(),
                );
            }
            cx.notify();
        }
        for op_id in active_ids {
            self.send_command(SftpCommand::SftpCancel { tab_id, op_id });
        }
    }
}

/// 任务执行所需的远端连接；方向决定用哪一端，缺失即视为入队缺陷。
///
/// 不再回退标签主连接或第一列：那会把跨列传输静默送到错误主机。
fn task_connection_id(task: &TransferTask) -> Option<ConnectionId> {
    match task.direction {
        TransferDirection::Upload => task.target_connection_id,
        TransferDirection::Download => task.source_connection_id,
        TransferDirection::Sync | TransferDirection::RemoteCopy => {
            task.target_connection_id.or(task.source_connection_id)
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
            mode: task.verify_mode,
            algorithm: task.checksum_algorithm,
        },
        (TransferKind::Directory, TransferDirection::Download) => SftpOperation::DownloadDir {
            remote_path: task.remote_path.clone(),
            local_path: task.local_path.clone(),
            resume,
            filter: filter.clone(),
            verify: task.verify,
            mode: task.verify_mode,
            algorithm: task.checksum_algorithm,
        },
        (TransferKind::File, TransferDirection::Upload) => SftpOperation::Upload {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            resume,
            verify: task.verify,
            mode: task.verify_mode,
            algorithm: task.checksum_algorithm,
        },
        (TransferKind::File, TransferDirection::Download) => SftpOperation::Download {
            remote_path: task.remote_path.clone(),
            local_path: task.local_path.clone(),
            resume,
            verify: task.verify,
            mode: task.verify_mode,
            algorithm: task.checksum_algorithm,
        },
        (TransferKind::Sync, TransferDirection::Sync) => SftpOperation::SyncDirs {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            items: task.sync_items.clone(),
            verify: task.verify,
            mode: task.verify_mode,
            algorithm: task.checksum_algorithm,
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

#[cfg(test)]
mod tests {
    use super::task_connection_id;
    use crate::sftp::{TransferMode, TransferTask};
    use std::path::PathBuf;
    use transom_core::{ConnectionId, TransferDirection};

    fn task(direction: TransferDirection) -> TransferTask {
        TransferTask::new(
            direction,
            PathBuf::from("/tmp/a"),
            "/remote/a".into(),
            None,
            TransferMode::Auto,
        )
    }

    /// 上传只认目标端连接：缺失时宁可失败，也不能回退到标签主连接。
    #[test]
    fn upload_requires_target_connection() {
        let mut upload = task(TransferDirection::Upload);
        assert_eq!(task_connection_id(&upload), None);
        let target = ConnectionId::new();
        upload.target_connection_id = Some(target);
        assert_eq!(task_connection_id(&upload), Some(target));
    }

    /// 下载只认源端连接，绝不能拿目标端字段兜底。
    #[test]
    fn download_requires_source_connection() {
        let mut download = task(TransferDirection::Download);
        assert_eq!(task_connection_id(&download), None);
        let source = ConnectionId::new();
        download.source_connection_id = Some(source);
        download.target_connection_id = Some(ConnectionId::new());
        assert_eq!(task_connection_id(&download), Some(source));
    }

    /// 跨远端互传两端都带连接，任一端可用即可调度（操作内部再校验两端）。
    #[test]
    fn remote_copy_accepts_either_endpoint() {
        let mut copy = task(TransferDirection::RemoteCopy);
        let source = ConnectionId::new();
        copy.source_connection_id = Some(source);
        assert_eq!(task_connection_id(&copy), Some(source));
    }
}
