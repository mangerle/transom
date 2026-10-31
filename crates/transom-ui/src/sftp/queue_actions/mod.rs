//! SFTP 传输队列调度、入队与批量控制。
//!
//! 设计原理（Why）：
//! - 实现初衷：负责 SFTP 传输队列的并发调度、任务分发与整列批量控制（暂停全部、恢复全部、取消全部）；
//! - 核心优势：
//!   1. 集中管理并发调度循环 `process_sftp_queue`；
//!   2. 规范化整批操作日志与命令发送顺序。

mod download;
pub(super) mod enqueue;
mod task_control;

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
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.queue_upload(tab_id, cx));
        }
    }

    /// 把本地列选中的一组路径上传到标签页主远端当前目录。
    pub(crate) fn enqueue_uploads(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let Some(view) = self.sftp_view(tab_id) else {
            return Err(rust_i18n::t!("sftp.queue_actions.no_remote_upload").to_string());
        };
        view.update(cx, |view, cx| view.enqueue_uploads(tab_id, paths, cx))
    }

    /// 把一组本地路径入队为上传任务并启动调度；队列状态与调度都归实体。
    pub(crate) fn enqueue_uploads_to(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        remote_dir: &str,
        target_connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let Some(view) = self.sftp_view(tab_id) else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        view.update(cx, |view, cx| {
            view.enqueue_uploads_to(tab_id, paths, remote_dir, target_connection_id, cx)
        })
    }

    /// 按并发上限启动待处理任务；调度本体在实体上，外壳只负责转发。
    pub(crate) fn process_sftp_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.process_queue(tab_id, cx));
        }
    }

    pub(crate) fn sftp_queue_clear_done(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
            panel.queue.tasks.retain(|task| {
                task.status != TransferStatus::Completed && task.status != TransferStatus::Cancelled
            });
        }
        cx.notify();
    }

    /// 循环切换并发传输数；增加时立即补位启动待处理任务。
    pub(crate) fn sftp_cycle_concurrency(&mut self, cx: &mut Context<Self>) {
        let current = self
            .state
            .settings
            .sftp
            .max_concurrent_transfers
            .clamp(1, 4);
        let next = if current >= 4 { 1 } else { current + 1 };
        self.state.settings.sftp.max_concurrent_transfers = next;
        if let Err(error) = self.state.persist_settings() {
            self.state.set_error(localized_error(&error));
        }
        self.state.status =
            Some(rust_i18n::t!("sftp.queue_actions.concurrency_set", count = next).to_string());
        if let Some(tab_id) = self.state.workspace.active_tab {
            self.process_sftp_queue(tab_id, cx);
        }
        cx.notify();
    }

    /// 一键重试传输队列中全部处于失败状态（`TransferStatus::Failed`）的任务。
    pub(crate) fn sftp_retry_all_failed(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut retried_count: usize = 0;
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
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
                panel.clear_queue_error();
                panel.set_status(
                    rust_i18n::t!("sftp.queue_actions.retried", count = retried_count).to_string(),
                );
            }
        }
        cx.notify();
        if retried_count > 0 {
            self.process_sftp_queue(tab_id, cx);
        }
    }

    /// 暂停全部活动与排队任务；队列收起时由状态栏调用。
    pub(crate) fn sftp_pause_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        log::debug!("暂停整列传输任务，窗格: {tab_id}");
        let mut active_ids: Vec<TransferId> = Vec::new();
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
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
        }
        cx.notify();
        for op_id in active_ids {
            self.state
                .send_command(SftpCommand::SftpCancel { tab_id, op_id });
        }
    }

    /// 继续全部暂停任务。
    pub(crate) fn sftp_resume_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut resumed = 0u32;
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
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
        }
        cx.notify();
        if resumed > 0 {
            self.process_sftp_queue(tab_id, cx);
        }
    }

    /// 取消全部活动、排队与暂停任务。
    pub(crate) fn sftp_cancel_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        log::debug!("取消整列传输任务，窗格: {tab_id}");
        let mut active_ids: Vec<TransferId> = Vec::new();
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
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
        }
        cx.notify();
        for op_id in active_ids {
            self.state
                .send_command(SftpCommand::SftpCancel { tab_id, op_id });
        }
    }
}

/// 任务执行所需的远端连接；方向决定用哪一端，缺失即视为入队缺陷。
///
/// 不再回退标签主连接或第一列：那会把跨列传输静默送到错误主机。
pub(super) fn task_connection_id(task: &TransferTask) -> Option<ConnectionId> {
    match task.direction {
        TransferDirection::Upload => task.target_connection_id,
        TransferDirection::Download => task.source_connection_id,
        TransferDirection::Sync | TransferDirection::RemoteCopy => {
            task.target_connection_id.or(task.source_connection_id)
        }
    }
}

pub(super) fn sftp_operation_for_task(
    task: &TransferTask,
    filter: &TransferFilter,
) -> Option<SftpOperation> {
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
