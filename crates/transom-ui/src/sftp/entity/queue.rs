//! 实体侧的传输队列调度。
//!
//! 调度只依赖面板状态与外壳设置：状态变更全部落在实体上，命令通过
//! 命令经面板事件交给外壳投递，外壳不再为一次补位调度去借还实体。

use super::*;
use crate::sftp::queue_actions::{sftp_operation_for_task, task_connection_id};
use crate::sftp::{TransferFailure, TransferStatus, TransferTask};
use transom_core::{SftpCommand, TransferFilter, VerifyOptions};

impl SftpPanelView {
    /// 按并发上限启动待处理任务；任一任务完成或取消后再次调用即可补位。
    pub(crate) fn process_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let state = &mut self.state;
        let filter_text = app_settings(cx).sftp.transfer_filter.clone();
        let max_concurrent = app_settings(cx).sftp.max_concurrent_transfers.clamp(1, 8);
        let filter = TransferFilter::from_text(&filter_text);
        let mut commands = Vec::new();
        while state.queue.active_transfers.len() < max_concurrent {
            let Some(index) = state
                .queue
                .tasks
                .iter()
                .position(|task| task.status == TransferStatus::Pending)
            else {
                break;
            };
            let task: TransferTask = state.queue.tasks[index].clone();
            let Some(connection_id) = task_connection_id(&task) else {
                state.queue.tasks[index].status =
                    TransferStatus::Failed(TransferFailure::Interrupted(
                        rust_i18n::t!("sftp.queue_actions.missing_connection").to_string(),
                    ));
                continue;
            };
            let Some(op) = sftp_operation_for_task(&task, &filter) else {
                state.queue.tasks[index].status =
                    TransferStatus::Failed(TransferFailure::Interrupted(
                        rust_i18n::t!("sftp.queue_actions.unsupported_task").to_string(),
                    ));
                continue;
            };
            state.queue.tasks[index].status = TransferStatus::Transferring;
            state.queue.active_transfers.insert(task.id);
            commands.push(SftpCommand::Sftp {
                tab_id,
                connection_id,
                op_id: task.id,
                op,
            });
        }

        if commands.is_empty() {
            state.recompute_progress();
            cx.notify();
            return;
        }
        state.clear_queue_error();
        state.recompute_progress();
        for command in commands {
            cx.emit(crate::sftp::SftpPanelEvent::SendCommand(command));
        }
        cx.notify();
    }
}

use crate::app::app_settings;
use crate::sftp::SftpTransferRequest;
use crate::sftp::queue_actions::enqueue::upload_tasks;
use crate::sftp::side_remote::SftpRemoteCopyParams;
use std::path::PathBuf;
use transom_core::TransferDirection;

impl SftpPanelView {
    /// 把本地列选中的一组路径上传到标签页主远端当前目录。
    pub(crate) fn queue_upload(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let paths = self.state.selected_local_paths();
        if let Err(message) = self.enqueue_uploads(tab_id, &paths, cx) {
            self.set_queue_panel_error(message, cx);
        }
    }

    /// 把本地列选中的一组路径上传到标签页主远端当前目录。
    pub(crate) fn enqueue_uploads(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let Some(connection_id) = self.state.main_connection else {
            return Err(rust_i18n::t!("sftp.queue_actions.no_remote_upload").to_string());
        };
        let remote_dir = self.state.remote.path.clone();
        self.enqueue_uploads_to(tab_id, paths, &remote_dir, connection_id, cx)
    }

    /// 把一组本地路径入队为上传任务并启动调度。
    ///
    /// 目标连接与远端目录显式传入：并列远端列的上传入口复用它，不能回退到
    /// 标签页主连接，否则文件会被传到另一台主机。
    pub(crate) fn enqueue_uploads_to(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        remote_dir: &str,
        target_connection_id: transom_core::ConnectionId,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        if paths.is_empty() {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        }
        let verify = VerifyOptions {
            enabled: app_settings(cx).sftp.verify_after_transfer,
            mode: app_settings(cx).sftp.verify_mode,
            algorithm: app_settings(cx).sftp.checksum_algorithm,
        };
        let tasks = upload_tasks(paths, remote_dir, verify, target_connection_id)?;
        let count = tasks.len();
        log::info!(
            "上传任务入队，窗格: {tab_id}，新增 {count} 项，目标目录: {remote_dir}，队列现有 {} 项，校验: {}",
            self.state.queue.tasks.len() + count,
            verify.enabled
        );
        self.state.queue.tasks.extend(tasks);
        self.state
            .set_status(rust_i18n::t!("sftp.upload_queued", count = count).to_string());
        self.process_queue(tab_id, cx);
        Ok(count)
    }

    fn set_queue_panel_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.state.set_queue_error(message);
        cx.notify();
    }

    /// 把单条上传/下载请求加入队列并立即尝试调度。
    pub(crate) fn enqueue_transfer(
        &mut self,
        request: SftpTransferRequest,
        cx: &mut Context<Self>,
    ) {
        let verify = VerifyOptions {
            enabled: app_settings(cx).sftp.verify_after_transfer,
            mode: app_settings(cx).sftp.verify_mode,
            algorithm: app_settings(cx).sftp.checksum_algorithm,
        };
        let mut task = if request.is_dir {
            TransferTask::new_directory(
                request.direction,
                request.local_path.clone(),
                request.remote_path.clone(),
                None,
            )
        } else {
            TransferTask::new(
                request.direction,
                request.local_path.clone(),
                request.remote_path.clone(),
                None,
            )
        };
        match request.direction {
            TransferDirection::Upload => {
                task.target_connection_id = Some(request.connection_id);
            }
            TransferDirection::Download => {
                task.source_connection_id = Some(request.connection_id);
            }
            _ => {}
        }
        task.verify = verify;
        self.state.queue.tasks.push(task);
        self.state.set_status(match request.direction {
            TransferDirection::Upload => rust_i18n::t!(
                "sftp.sidebar.upload_queued_path",
                path = request.remote_path.as_str()
            )
            .to_string(),
            TransferDirection::Download => rust_i18n::t!(
                "sftp.sidebar.download_queued_path",
                path = request.remote_path.as_str()
            )
            .to_string(),
            _ => rust_i18n::t!(
                "sftp.sidebar.transfer_queued_path",
                path = request.remote_path.as_str()
            )
            .to_string(),
        });
        self.process_queue(request.tab_id, cx);
    }

    /// 跨远端互传任务入队。
    pub(crate) fn enqueue_remote_transfer(
        &mut self,
        tab_id: TabId,
        params: SftpRemoteCopyParams,
        cx: &mut Context<Self>,
    ) {
        let src_path = params.source_path.clone();
        let tgt_path = params.target_path.clone();
        let task = TransferTask::new_remote_copy(
            params.source_connection_id,
            params.source_path,
            params.target_connection_id,
            params.target_path,
            None,
            params.is_dir,
        );
        self.state.queue.tasks.push(task);
        self.state.set_status(
            rust_i18n::t!(
                "sftp.sidebar.remote_copy_queued",
                src = src_path.as_str(),
                tgt = tgt_path.as_str()
            )
            .to_string(),
        );
        self.process_queue(tab_id, cx);
    }
}
