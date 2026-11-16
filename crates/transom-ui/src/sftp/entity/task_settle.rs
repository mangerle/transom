//! SFTP 面板实体完成与失败落定逻辑。
//!
//! 设计原理（Why）：传输任务状态流转、目录刷新目标计算、暂态提示与队列清理
//! 属于面板实体的内聚领域逻辑。收纳在此后，外壳事件处理器仅需路由与触发桌面系统通知。

use super::SftpPanelView;
use crate::palette::localized_error;
use crate::sftp::{TransferFailure, TransferStatus, format_size, parent_remote};
use gpui_kit::*;
use transom_core::{AppError, ConnectionId, TransferDirection, TransferId};

/// SFTP 完成后的刷新与提示计划。
#[derive(Default)]
pub(crate) struct SftpFinishOutcome {
    pub(crate) refresh: Option<String>,
    pub(crate) refresh_side: Option<(ConnectionId, String)>,
    pub(crate) refresh_local: bool,
    pub(crate) notify_info: Option<(TransferDirection, String, u64)>,
}

/// SFTP 失败后的通知与队列错误计划。
#[derive(Default)]
pub(crate) struct SftpFailureOutcome {
    pub(crate) failed_notify: Option<(String, String)>,
}

/// 单个传输任务完成后的刷新与通知计划。
#[derive(Default)]
struct TransferCompletion {
    refresh_side: Option<(ConnectionId, String)>,
    refresh_local: bool,
    notify_info: Option<(TransferDirection, String, u64)>,
}

impl SftpPanelView {
    /// 落定单个传输或非传输操作完成。
    pub(crate) fn settle_transfer_finished(
        &mut self,
        connection_id: ConnectionId,
        op_id: TransferId,
        transferred: Option<u64>,
        skipped: u64,
        verified: bool,
        cx: &mut Context<Self>,
    ) -> SftpFinishOutcome {
        let mut refresh = None;
        let mut refresh_side = None;
        let mut refresh_local = false;
        let mut notify_info = None;
        let main_conn = self.state.main_connection;

        let completion = self
            .state
            .queue
            .tasks
            .iter_mut()
            .find(|t| t.id == op_id)
            .map(|task| complete_transfer_task(task, transferred, main_conn));

        if let Some(completion) = completion {
            refresh_local = completion.refresh_local;
            refresh_side = completion.refresh_side;
            notify_info = completion.notify_info;
            self.state.queue.active_transfers.remove(&op_id);
            self.state.recompute_progress();
            if self.state.queue.active_transfers.is_empty() {
                let base = match transferred {
                    Some(bytes) => format!(
                        "{} ({})",
                        rust_i18n::t!("sftp.transfer_done"),
                        format_size(bytes)
                    ),
                    None => rust_i18n::t!("sftp.operation_done").to_string(),
                };
                let mut message = base;
                if skipped > 0 {
                    message.push_str(&format!(
                        " | {}: {skipped}",
                        rust_i18n::t!("sftp.skipped_items")
                    ));
                }
                if verified {
                    message.push_str(&format!(" | {}", rust_i18n::t!("sftp.sha256_verified")));
                }
                self.set_transient_status(message, cx);
                self.state.clear_queue_error();
                refresh = Some(self.state.remote.path.clone());
            }
        } else {
            let pane_path = self
                .state
                .remote_pane_of_connection(connection_id, main_conn)
                .and_then(|target| self.state.remote_pane(target))
                .map(|pane| pane.path.clone());
            if let Some(path) = pane_path {
                refresh_side = Some((connection_id, path));
            }
            let done_msg = rust_i18n::t!("sftp.operation_done").to_string();
            self.set_transient_status(done_msg, cx);
        }

        SftpFinishOutcome {
            refresh,
            refresh_side,
            refresh_local,
            notify_info,
        }
    }

    /// 落定单个传输或非传输操作失败。
    pub(crate) fn settle_transfer_failed(
        &mut self,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: &AppError,
        cx: &mut Context<Self>,
    ) -> SftpFailureOutcome {
        let err_msg = localized_error(error);
        let mut failed_notify = None;
        let mut failed_transfer = false;

        if let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id) {
            failed_transfer = true;
            task.status =
                TransferStatus::Failed(TransferFailure::from_error(error, err_msg.clone()));
            let file_name = task
                .local_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| task.remote_path.clone());
            failed_notify = Some((file_name, err_msg.clone()));
        }

        if failed_transfer {
            self.state.queue.active_transfers.remove(&op_id);
            self.state.recompute_progress();
            self.set_transient_queue_error(err_msg.clone(), cx);
        } else {
            let main_conn = self.state.main_connection;
            if let Some(target) = self
                .state
                .remote_pane_of_connection(connection_id, main_conn)
                && let Some(pane) = self.state.remote_pane_mut(target)
            {
                pane.loading = false;
                pane.error = Some(err_msg);
            }
        }

        SftpFailureOutcome { failed_notify }
    }
}

/// 落定单个传输任务：状态、进度目标与通知信息。
fn complete_transfer_task(
    task: &mut crate::sftp::TransferTask,
    transferred: Option<u64>,
    main_conn: Option<ConnectionId>,
) -> TransferCompletion {
    let mut completion = TransferCompletion::default();
    task.status = TransferStatus::Completed;
    task.speed_bps = None;
    task.eta_secs = None;
    if let Some(bytes) = transferred {
        task.transferred = bytes;
    }
    if task.direction == TransferDirection::Download || task.direction == TransferDirection::Sync {
        completion.refresh_local = true;
    }
    if task.direction == TransferDirection::RemoteCopy
        && let (Some(target_conn), Some(target_path)) =
            (task.target_connection_id, &task.target_remote_path)
    {
        let dir = target_path
            .rsplit_once('/')
            .map(|(d, _)| if d.is_empty() { "/" } else { d })
            .unwrap_or("/");
        completion.refresh_side = Some((target_conn, dir.to_string()));
    }
    if task.direction == TransferDirection::Upload
        && let Some(target_conn) = task.target_connection_id
        && Some(target_conn) != main_conn
    {
        completion.refresh_side = Some((target_conn, parent_remote(&task.remote_path)));
    }
    if task.direction == TransferDirection::Sync
        && let Some(target_conn) = task.target_connection_id
        && Some(target_conn) != main_conn
    {
        completion.refresh_side = Some((target_conn, task.remote_path.clone()));
    }
    let file_name = if task.direction == TransferDirection::RemoteCopy {
        task.target_remote_path
            .clone()
            .unwrap_or_else(|| task.remote_path.clone())
    } else {
        task.local_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| task.remote_path.clone())
    };
    completion.notify_info = Some((task.direction, file_name, task.transferred));
    completion
}
