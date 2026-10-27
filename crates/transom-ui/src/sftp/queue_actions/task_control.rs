//! SFTP 单任务控制操作（暂停、恢复、取消、置顶与移除）。
//!
//! 设计原理（Why）：
//! - 实现初衷：为传输队列中针对单个任务的精确交互操作提供独立逻辑单元；
//! - 核心优势：集中管理状态流转与命令发送（SftpCancel），明确细粒度职责。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{AppCommand, TabId, TransferId, TransferMode, TransferStatus};

impl AppShell {
    pub(crate) fn sftp_pause_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let mut needs_cancel = false;
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id)
                && task.status == TransferStatus::Transferring
            {
                task.status = TransferStatus::Paused;
                needs_cancel = true;
            }
            panel.active_transfers.remove(&op_id);
            panel.recompute_progress();
            if panel.active_transfers.is_empty() {
                panel.status = Some("传输已暂停".to_string());
            }
            cx.notify();
        }
        // 发送在 panel 借用结束后：send_command 需要整体可变借用 self
        if needs_cancel {
            self.send_command(AppCommand::SftpCancel { tab_id, op_id });
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
        log::debug!("取消传输任务，窗格: {tab_id}，任务: {op_id}");
        let mut should_cancel = false;
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                if task.status == TransferStatus::Transferring {
                    should_cancel = true;
                }
                task.status = TransferStatus::Cancelled;
            }
            panel.active_transfers.remove(&op_id);
            panel.recompute_progress();
            if panel.active_transfers.is_empty() {
                panel.status = Some("已取消传输".to_string());
            }
            cx.notify();
        }
        // 发送在 panel 借用结束后：send_command 需要整体可变借用 self
        if should_cancel {
            self.send_command(AppCommand::SftpCancel { tab_id, op_id });
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
        let mut needs_cancel = false;
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
                needs_cancel = true;
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
        // 发送在 panel 借用结束后：send_command 需要整体可变借用 self
        if needs_cancel {
            self.send_command(AppCommand::SftpCancel { tab_id, op_id });
        }
        self.process_sftp_queue(tab_id, cx);
    }
}
