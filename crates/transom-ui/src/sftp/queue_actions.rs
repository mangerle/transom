//! SFTP 传输队列调度、上传下载入队、暂停恢复取消与置顶操作方法。

use super::join_remote;
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
    ///
    /// 面板里的"上传"按钮与终端拖放共用这条路径：两者的差别只在路径来源
    /// （本地栏选中项 vs 拖入的文件），任务构造、状态文案与调度都必须一致。
    /// 返回 `Err(文案)` 表示整批未入队，由调用方决定把文案写到面板还是错误栏。
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
        // 取消要留在日志里：远端可能已经写出一个半成品文件，事后"文件哪来的"
        // 只能靠这条记录对上时间线。按 debug 记是因为它由用户逐个点击触发，
        // 不属于全局生命周期节点。
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

    /// 暂停全部活动与排队任务；队列收起时由状态栏调用。
    pub(crate) fn sftp_pause_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        log::debug!("暂停整列传输任务，窗格: {tab_id}");
        // 借用 panel 前先收集活动任务 id：发送取消命令需要整体可变借用 self
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
        // 发送在 panel 借用结束后：send_command 需要整体可变借用 self
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
        // 借用 panel 前先收集活动任务 id：发送取消命令需要整体可变借用 self
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
        // 发送在 panel 借用结束后
        for op_id in active_ids {
            self.send_command(AppCommand::SftpCancel { tab_id, op_id });
        }
    }
}

/// 把本地路径列表转成上传任务；目录与文件走不同的构造分支。
///
/// 设计原理（Why）：入队逻辑必须与"上传按钮"完全一致，因此抽成纯函数——
/// 它只读文件系统元数据，不碰界面状态，可单测覆盖目录/文件分派与取名失败。
///
/// 单个路径取不到文件名（如拖入文件系统根目录或含无效 UTF-8 的名称）时整批
/// 拒绝：部分入队会让用户以为"传了 3 个"，实际只传了 2 个。
fn upload_tasks(
    paths: &[PathBuf],
    remote_dir: &str,
    resume_enabled: bool,
    verify: bool,
) -> Result<Vec<TransferTask>, String> {
    let mode = if resume_enabled {
        TransferMode::Resume
    } else {
        TransferMode::Overwrite
    };
    let mut tasks = Vec::with_capacity(paths.len());
    for local_path in paths {
        let Some(file_name) = local_path.file_name().and_then(|name| name.to_str()) else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let remote_path = join_remote(remote_dir, file_name);
        let is_directory = std::fs::symlink_metadata(local_path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        let size = if is_directory {
            None
        } else {
            std::fs::metadata(local_path).map(|meta| meta.len()).ok()
        };
        let mut task = if is_directory {
            TransferTask::new_directory(
                TransferDirection::Upload,
                local_path.clone(),
                remote_path,
                size,
                mode,
            )
        } else {
            TransferTask::new(
                TransferDirection::Upload,
                local_path.clone(),
                remote_path,
                size,
                mode,
            )
        };
        task.verify = verify;
        tasks.push(task);
    }
    Ok(tasks)
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
