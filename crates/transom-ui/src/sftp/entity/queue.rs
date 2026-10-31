//! 实体侧的传输队列调度。
//!
//! 调度只依赖面板状态与外壳设置：状态变更全部落在实体上，命令通过
//! `with_shell_parts` 回交外壳投递。外壳不再为一次补位调度去借还实体。

use super::*;
use crate::sftp::queue_actions::{is_verify_failure, sftp_operation_for_task, task_connection_id};
use crate::sftp::{TransferStatus, TransferTask};
use transom_core::{SftpCommand, TransferFilter, TransferId};

impl SftpPanelView {
    /// 按并发上限启动待处理任务；任一任务完成或取消后再次调用即可补位。
    pub(crate) fn process_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let SftpPanelView { state, app } = self;
        let Some((filter_text, max_concurrent)) = read_shell_parts(app, cx, |shell| {
            (
                shell.state.settings.sftp.transfer_filter.clone(),
                shell
                    .state
                    .settings
                    .sftp
                    .max_concurrent_transfers
                    .clamp(1, 8),
            )
        }) else {
            return;
        };
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
                state.queue.tasks[index].status = TransferStatus::Failed(
                    rust_i18n::t!("sftp.queue_actions.missing_connection").to_string(),
                );
                continue;
            };
            let Some(op) = sftp_operation_for_task(&task, &filter) else {
                state.queue.tasks[index].status = TransferStatus::Failed(
                    rust_i18n::t!("sftp.queue_actions.unsupported_task").to_string(),
                );
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
        with_shell_parts(app, cx, |shell, _| {
            for command in commands {
                shell.state.send_command(command);
            }
        });
        cx.notify();
    }
}

use crate::sftp::queue_actions::enqueue::upload_tasks;
use crate::sftp::side_remote::SftpRemoteCopyParams;
use crate::sftp::{SftpTransferRequest, TransferMode};
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
        let app = self.app.clone();
        let Some(connection_id) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
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
        let app = self.app.clone();
        let Some((verify, verify_mode, algorithm)) = read_shell_parts(&app, cx, |shell| {
            (
                shell.state.settings.sftp.verify_after_transfer,
                shell.state.settings.sftp.verify_mode,
                shell.state.settings.sftp.checksum_algorithm,
            )
        }) else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let tasks = upload_tasks(
            paths,
            remote_dir,
            self.state.queue.resume_enabled,
            verify,
            verify_mode,
            algorithm,
            target_connection_id,
        )?;
        let count = tasks.len();
        log::info!(
            "上传任务入队，窗格: {tab_id}，新增 {count} 项，目标目录: {remote_dir}，队列现有 {} 项，校验: {verify}",
            self.state.queue.tasks.len() + count
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
        let app = self.app.clone();
        let Some((verify, verify_mode, checksum_algorithm)) = read_shell_parts(&app, cx, |shell| {
            (
                shell.state.settings.sftp.verify_after_transfer,
                shell.state.settings.sftp.verify_mode,
                shell.state.settings.sftp.checksum_algorithm,
            )
        }) else {
            return;
        };
        let mode = if self.state.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut task = if request.is_dir {
            TransferTask::new_directory(
                request.direction,
                request.local_path.clone(),
                request.remote_path.clone(),
                None,
                mode,
            )
        } else {
            TransferTask::new(
                request.direction,
                request.local_path.clone(),
                request.remote_path.clone(),
                None,
                mode,
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
        task.verify_mode = verify_mode;
        task.checksum_algorithm = checksum_algorithm;
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

use crate::palette::localized_error;

impl SftpPanelView {
    /// 清空已完成与已取消的队列项。
    pub(crate) fn queue_clear_done(&mut self, cx: &mut Context<Self>) {
        self.state.queue.tasks.retain(|task| {
            task.status != TransferStatus::Completed && task.status != TransferStatus::Cancelled
        });
        cx.notify();
    }

    /// 循环切换并发传输数；增加时立即补位启动待处理任务。
    pub(crate) fn cycle_concurrency(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let Some(current) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .settings
                .sftp
                .max_concurrent_transfers
                .clamp(1, 4)
        }) else {
            return;
        };
        let next = if current >= 4 { 1 } else { current + 1 };
        with_shell_parts(&app, cx, |shell, cx| {
            shell.state.settings.sftp.max_concurrent_transfers = next;
            if let Err(error) = shell.state.persist_settings() {
                shell.state.set_error(localized_error(&error));
            }
            shell.state.status =
                Some(rust_i18n::t!("sftp.queue_actions.concurrency_set", count = next).to_string());
            cx.notify();
        });
        self.process_queue(tab_id, cx);
    }

    /// 一键重试队列中全部失败任务。
    pub(crate) fn retry_all_failed(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let resume_enabled = self.state.queue.resume_enabled;
        let mut retried_count: usize = 0;
        for task in &mut self.state.queue.tasks {
            if let TransferStatus::Failed(ref reason) = task.status {
                let is_verify_failure = is_verify_failure(reason);
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
        if retried_count == 0 {
            return;
        }
        self.state.clear_queue_error();
        self.state.set_status(
            rust_i18n::t!("sftp.queue_actions.retried", count = retried_count).to_string(),
        );
        cx.notify();
        self.process_queue(tab_id, cx);
    }

    /// 暂停全部活动与排队任务。
    pub(crate) fn pause_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        log::debug!("暂停整列传输任务，窗格: {tab_id}");
        let active_ids: Vec<TransferId> =
            self.state.queue.active_transfers.iter().copied().collect();
        let mut paused = 0u32;
        for task in &mut self.state.queue.tasks {
            match task.status {
                TransferStatus::Transferring | TransferStatus::Pending => {
                    task.status = TransferStatus::Paused;
                    paused = paused.saturating_add(1);
                }
                _ => {}
            }
        }
        self.state.queue.active_transfers.clear();
        self.state.recompute_progress();
        if paused > 0 {
            self.state
                .set_status(rust_i18n::t!("sftp.queue_actions.paused", count = paused).to_string());
        }
        self.cancel_remote_ops(tab_id, active_ids, cx);
        cx.notify();
    }

    /// 继续全部暂停任务。
    pub(crate) fn resume_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let mut resumed = 0u32;
        for task in &mut self.state.queue.tasks {
            if task.status == TransferStatus::Paused {
                task.status = TransferStatus::Pending;
                task.mode = TransferMode::Resume;
                resumed = resumed.saturating_add(1);
            }
        }
        if resumed == 0 {
            return;
        }
        self.state
            .set_status(rust_i18n::t!("sftp.queue_actions.resumed", count = resumed).to_string());
        cx.notify();
        self.process_queue(tab_id, cx);
    }

    /// 取消全部活动、排队与暂停任务。
    pub(crate) fn cancel_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        log::debug!("取消整列传输任务，窗格: {tab_id}");
        let active_ids: Vec<TransferId> =
            self.state.queue.active_transfers.iter().copied().collect();
        let mut cancelled = 0u32;
        for task in &mut self.state.queue.tasks {
            if matches!(
                task.status,
                TransferStatus::Transferring | TransferStatus::Pending | TransferStatus::Paused
            ) {
                task.status = TransferStatus::Cancelled;
                cancelled = cancelled.saturating_add(1);
            }
        }
        self.state.queue.active_transfers.clear();
        self.state.recompute_progress();
        if cancelled > 0 {
            self.state.set_status(
                rust_i18n::t!("sftp.queue_actions.cancelled", count = cancelled).to_string(),
            );
        }
        self.cancel_remote_ops(tab_id, active_ids, cx);
        cx.notify();
    }

    /// 把一批被中断任务的取消命令回交外壳投递。
    fn cancel_remote_ops(&self, tab_id: TabId, op_ids: Vec<TransferId>, cx: &mut Context<Self>) {
        if op_ids.is_empty() {
            return;
        }
        let app = self.app.clone();
        with_shell_parts(&app, cx, |shell, _| {
            for op_id in op_ids {
                shell
                    .state
                    .send_command(SftpCommand::SftpCancel { tab_id, op_id });
            }
        });
    }
}

impl SftpPanelView {
    /// 暂停单个任务；正在传输的先下发远端取消，再从调度集合摘除。
    pub(crate) fn pause_task(&mut self, tab_id: TabId, op_id: TransferId, cx: &mut Context<Self>) {
        let mut needs_cancel = false;
        if let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id)
            && task.status == TransferStatus::Transferring
        {
            task.status = TransferStatus::Paused;
            needs_cancel = true;
        }
        self.state.queue.active_transfers.remove(&op_id);
        self.state.recompute_progress();
        if self.state.queue.active_transfers.is_empty() {
            self.state
                .set_status(rust_i18n::t!("sftp.queue_actions.paused_one").to_string());
        }
        if needs_cancel {
            self.cancel_remote_ops(tab_id, vec![op_id], cx);
        }
        self.process_queue(tab_id, cx);
    }

    /// 继续单个暂停或失败任务；校验失败强制从头覆盖传输。
    pub(crate) fn resume_task(&mut self, tab_id: TabId, op_id: TransferId, cx: &mut Context<Self>) {
        if let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id)
            && (task.status == TransferStatus::Paused
                || matches!(task.status, TransferStatus::Failed(_)))
        {
            let is_verify_failure = match &task.status {
                TransferStatus::Failed(reason) => is_verify_failure(reason),
                _ => false,
            };
            task.status = TransferStatus::Pending;
            if is_verify_failure {
                task.mode = TransferMode::Overwrite;
                task.transferred = 0;
            } else {
                task.mode = TransferMode::Resume;
            }
        }
        self.process_queue(tab_id, cx);
    }

    /// 取消单个任务。
    pub(crate) fn cancel_task(&mut self, tab_id: TabId, op_id: TransferId, cx: &mut Context<Self>) {
        log::debug!("取消传输任务，窗格: {tab_id}，任务: {op_id}");
        let mut should_cancel = false;
        if let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id) {
            if task.status == TransferStatus::Transferring {
                should_cancel = true;
            }
            task.status = TransferStatus::Cancelled;
        }
        self.state.queue.active_transfers.remove(&op_id);
        self.state.recompute_progress();
        if self.state.queue.active_transfers.is_empty() {
            self.state
                .set_status(rust_i18n::t!("sftp.queue_actions.cancelled_one").to_string());
        }
        if should_cancel {
            self.cancel_remote_ops(tab_id, vec![op_id], cx);
        }
        self.process_queue(tab_id, cx);
    }

    /// 从队列移除单个任务（结束态任务的清理入口）。
    pub(crate) fn remove_task(&mut self, op_id: TransferId, cx: &mut Context<Self>) {
        self.state.queue.active_transfers.remove(&op_id);
        self.state.queue.tasks.retain(|task| task.id != op_id);
        self.state.recompute_progress();
        cx.notify();
    }

    /// 把单个任务置顶；在途任务先取消远端操作再重新排队。
    pub(crate) fn prioritize_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let mut needs_cancel = false;
        if self.state.queue.active_transfers.contains(&op_id)
            && let Some(active_task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id)
            && active_task.status == TransferStatus::Transferring
        {
            needs_cancel = true;
            active_task.status = TransferStatus::Pending;
            active_task.mode = TransferMode::Resume;
            self.state.queue.active_transfers.remove(&op_id);
        }
        if let Some(pos) = self.state.queue.tasks.iter().position(|t| t.id == op_id) {
            let mut task = self.state.queue.tasks.remove(pos);
            task.status = TransferStatus::Pending;
            if task.transferred > 0 {
                task.mode = TransferMode::Resume;
            }
            self.state.queue.tasks.insert(0, task);
            self.state
                .set_status(rust_i18n::t!("sftp.queue_actions.prioritized").to_string());
        }
        self.state.recompute_progress();
        if needs_cancel {
            self.cancel_remote_ops(tab_id, vec![op_id], cx);
        }
        self.process_queue(tab_id, cx);
    }
}
