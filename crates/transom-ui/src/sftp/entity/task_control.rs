//! 实体侧的队列控制与单任务操作。
//!
//! 状态流转在实体完成，远端取消命令经 with_shell_parts 投递。

use super::*;
use crate::palette::localized_error;
use crate::sftp::queue_actions::is_verify_failure;
use crate::sftp::{TransferMode, TransferStatus};
use transom_core::{SftpCommand, TransferId};

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
