//! SFTP 传输队列调度、上传下载入队、暂停恢复取消与置顶操作方法。

use super::join_remote;
use crate::app::AppShell;
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
        let Some(local_path) = panel.local_selected.clone() else {
            panel.error = Some(rust_i18n::t!("sftp.upload_file").to_string());
            cx.notify();
            return;
        };
        let Some(file_name) = local_path.file_name().and_then(|n| n.to_str()) else {
            panel.error = Some(rust_i18n::t!("sftp.upload_file").to_string());
            cx.notify();
            return;
        };
        let remote_path = join_remote(&panel.path, file_name);
        let is_directory = local_path.is_dir();
        let size = if is_directory {
            None
        } else {
            std::fs::metadata(&local_path).map(|m| m.len()).ok()
        };
        let mode = if panel.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
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
        panel.queue.push(task);
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
        let Some(selected) = panel.selected.clone() else {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        };
        let Some(remote_entry) = panel.entries.iter().find(|e| e.path == selected) else {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        };
        let local_path = panel.local_path.join(&remote_entry.name);
        let mode = if panel.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
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
        panel.queue.push(task);
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn process_sftp_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let filter = TransferFilter::from_text(&self.settings.sftp.transfer_filter);
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        if panel.active_op.is_some() {
            return;
        }
        let pending_index = panel
            .queue
            .iter()
            .position(|task| task.status == TransferStatus::Pending);
        let Some(index) = pending_index else {
            return;
        };

        panel.queue[index].status = TransferStatus::Transferring;
        let task = panel.queue[index].clone();
        panel.active_op = Some(task.id);
        panel.progress = Some((task.transferred, task.size));
        panel.error = None;

        let resume = task.mode != TransferMode::Overwrite;
        let op = match (task.kind, task.direction) {
            (TransferKind::Directory, TransferDirection::Upload) => SftpOperation::UploadDir {
                local_path: task.local_path,
                remote_path: task.remote_path,
                resume,
                filter,
                verify: task.verify,
            },
            (TransferKind::Directory, TransferDirection::Download) => SftpOperation::DownloadDir {
                remote_path: task.remote_path,
                local_path: task.local_path,
                resume,
                filter,
                verify: task.verify,
            },
            (TransferKind::File, TransferDirection::Upload) => SftpOperation::Upload {
                local_path: task.local_path,
                remote_path: task.remote_path,
                resume,
                verify: task.verify,
            },
            (TransferKind::File, TransferDirection::Download) => SftpOperation::Download {
                remote_path: task.remote_path,
                local_path: task.local_path,
                resume,
                verify: task.verify,
            },
            (TransferKind::Sync, TransferDirection::Sync) => SftpOperation::SyncDirs {
                local_path: task.local_path,
                remote_path: task.remote_path,
                items: task.sync_items,
                verify: task.verify,
            },
            _ => {
                panel.queue[index].status =
                    TransferStatus::Failed("不支持的传输任务类型组合".to_string());
                panel.active_op = None;
                panel.progress = None;
                cx.notify();
                return;
            }
        };

        let _ = self.services.command_tx.send(AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id: task.id,
            op,
        });
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
            if panel.active_op == Some(op_id) {
                panel.active_op = None;
                panel.progress = None;
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
            if panel.active_op == Some(op_id) {
                panel.active_op = None;
                panel.progress = None;
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
            panel.queue.retain(|task| task.id != op_id);
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
            if let Some(active_id) = panel.active_op
                && active_id != op_id
                && let Some(active_task) = panel.queue.iter_mut().find(|t| t.id == active_id)
                && active_task.status == TransferStatus::Transferring
            {
                let _ = self.services.command_tx.send(AppCommand::SftpCancel {
                    tab_id,
                    op_id: active_id,
                });
                active_task.status = TransferStatus::Pending;
                active_task.mode = TransferMode::Resume;
                panel.active_op = None;
                panel.progress = None;
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
            cx.notify();
        }
        self.process_sftp_queue(tab_id, cx);
    }
}
