//! SFTP 单任务控制操作（暂停、恢复、取消、置顶与移除）。
//!
//! 设计原理（Why）：
//! - 实现初衷：为传输队列中针对单个任务的精确交互操作提供独立逻辑单元；
//! - 核心优势：集中管理状态流转与命令发送（SftpCancel），明确细粒度职责。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{TabId, TransferId};

impl AppShell {
    pub(crate) fn sftp_pause_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.pause_task(tab_id, op_id, cx));
        }
    }

    pub(crate) fn sftp_resume_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.resume_task(tab_id, op_id, cx));
        }
    }

    pub(crate) fn sftp_cancel_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.cancel_task(tab_id, op_id, cx));
        }
    }

    pub(crate) fn sftp_remove_task(
        &mut self,
        tab_id: TabId,
        op_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.remove_task(op_id, cx));
        }
    }
}
