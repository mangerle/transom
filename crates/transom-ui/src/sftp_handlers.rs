//! SFTP 事件的域处理：目录回填、进度、完成/失败、属性与预览。
//!
//! 与 [`crate::event_handlers`] 的分工：本文件只处理 SFTP 传输相关事件，
//! 连接/认证与遥测事件在另一处；拆分让每个文件保持在工程规范的体量内。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::sftp::TransferFailure;
use crate::sftp::TransferStatus;
use crate::sftp::{format_size, parent_remote, send_desktop_notification};
use gpui_kit::*;
use transom_core::{SftpAppEvent, TransferDirection};

/// SFTP 完成事件中与队列落定相关的输入。
struct SftpFinishInput {
    tab_id: transom_core::TabId,
    connection_id: transom_core::ConnectionId,
    op_id: transom_core::TransferId,
    transferred: Option<u64>,
    skipped: u64,
    verified: bool,
}

/// SFTP 完成后的刷新与提示计划。
#[derive(Default)]
struct SftpFinishOutcome {
    refresh: Option<String>,
    refresh_side: Option<(transom_core::ConnectionId, String)>,
    refresh_local: bool,
    notify_info: Option<(TransferDirection, String, u64)>,
    status_message: Option<String>,
}

/// 单个传输任务完成后的刷新与通知计划。
#[derive(Default)]
struct TransferCompletion {
    refresh_side: Option<(transom_core::ConnectionId, String)>,
    refresh_local: bool,
    notify_info: Option<(TransferDirection, String, u64)>,
}

/// 落定单个传输任务：状态、进度目标与通知信息。
fn complete_transfer_task(
    task: &mut crate::sftp::TransferTask,
    transferred: Option<u64>,
    main_conn: Option<transom_core::ConnectionId>,
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
    // 上传落到并列远端列时刷新该列目录：完成路径原本只刷新主远端，
    // 挂载列看不到刚上传的文件。上传到主远端仍由下方统一刷新覆盖。
    if task.direction == TransferDirection::Upload
        && let Some(target_conn) = task.target_connection_id
        && Some(target_conn) != main_conn
    {
        completion.refresh_side = Some((target_conn, parent_remote(&task.remote_path)));
    }
    // 并列远端列的目录同步：任务记录的是远端根目录，完成后直接刷新该列根。
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

impl AppShell {
    /// SFTP 传输完成：落定任务状态、刷新受影响的目录并派发桌面通知。
    pub(crate) fn on_sftp_finished(&mut self, event: SftpAppEvent, cx: &mut Context<Self>) -> bool {
        let SftpAppEvent::SftpFinished {
            tab_id,
            connection_id,
            op_id,
            transferred,
            skipped,
            verified,
        } = event
        else {
            return true;
        };
        let outcome = self.apply_sftp_finish(
            SftpFinishInput {
                tab_id,
                connection_id,
                op_id,
                transferred,
                skipped,
                verified,
            },
            cx,
        );
        let SftpFinishOutcome {
            refresh,
            refresh_side,
            refresh_local,
            notify_info,
            status_message,
        } = outcome;
        if let Some(message) = status_message {
            self.set_sftp_transient_status(tab_id, message, cx);
        }
        // 任务落定，浮层据此决定继续常驻还是进入 5 秒停留窗口。
        self.sync_drop_overlay(tab_id, cx);
        if let Some((dir, file_name, bytes)) = notify_info {
            let action = match dir {
                TransferDirection::Upload => rust_i18n::t!("sftp.handlers.notify_upload_done"),
                TransferDirection::Download => rust_i18n::t!("sftp.handlers.notify_download_done"),
                TransferDirection::Sync => rust_i18n::t!("sftp.handlers.notify_sync_done"),
                TransferDirection::RemoteCopy => {
                    rust_i18n::t!("sftp.handlers.notify_remote_copy_done")
                }
            };
            send_desktop_notification(
                &format!("Transom SFTP {action}"),
                &format!("{file_name} ({})", format_size(bytes)),
            );
        }
        if refresh_local {
            self.sftp_local_refresh(tab_id, cx);
        }
        if let Some(path) = refresh {
            self.request_sftp_list(tab_id, path, cx);
        }
        if let Some((target_conn, target_dir)) = refresh_side {
            self.request_sftp_list_for_connection(tab_id, target_conn, target_dir, cx);
        }
        self.process_sftp_queue(tab_id, cx);
        true
    }

    /// 应用 SFTP 完成：落定队列任务、计算需要刷新的列与状态文案。
    fn apply_sftp_finish(
        &mut self,
        finish: SftpFinishInput,
        cx: &mut Context<Self>,
    ) -> SftpFinishOutcome {
        let SftpFinishInput {
            tab_id,
            connection_id,
            op_id,
            transferred,
            skipped,
            verified,
        } = finish;
        let mut refresh = None;
        let mut refresh_side = None;
        let mut refresh_local = false;
        let mut notify_info: Option<(TransferDirection, String, u64)> = None;
        let mut status_message: Option<String> = None;
        let main_conn = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id);
        if let Some(mut panel) = self.sftp_view_mut_for_connection(tab_id, connection_id, cx) {
            let completion = panel
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
                panel.state.queue.active_transfers.remove(&op_id);
                panel.state.recompute_progress();
                if panel.state.queue.active_transfers.is_empty() {
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
                    status_message = Some(message);
                    panel.state.clear_queue_error();
                    refresh = Some(panel.state.remote.path.clone());
                }
            } else {
                // 非传输操作：按连接找到目标列并刷新它自己的当前目录；
                // 主远端与并列列不再各写一条完成分支。
                let pane_path = panel
                    .state
                    .remote_pane_of_connection(connection_id, main_conn)
                    .and_then(|target| panel.state.remote_pane(target))
                    .map(|pane| pane.path.clone());
                if let Some(path) = pane_path {
                    refresh_side = Some((connection_id, path));
                }
                status_message = Some(rust_i18n::t!("sftp.operation_done").to_string());
            }
        }
        SftpFinishOutcome {
            refresh,
            refresh_side,
            refresh_local,
            notify_info,
            status_message,
        }
    }

    /// SFTP 失败：同步模态失败自行重绘并返回；传输失败落定任务并通知。
    pub(crate) fn on_sftp_failed(&mut self, event: SftpAppEvent, cx: &mut Context<Self>) -> bool {
        let SftpAppEvent::SftpFailed {
            tab_id,
            connection_id,
            op_id,
            error,
        } = event
        else {
            return true;
        };
        if let Some(modal) = self.state.sftp_modals.sync.as_mut()
            && modal.op_id == op_id
            && modal.connection_id == connection_id
        {
            modal.scanning = false;
            modal.error = Some(localized_error(&error));
            cx.notify();
            return false;
        }
        let mut failed_notify: Option<(String, String)> = None;
        let main_conn = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id);
        if let Some(mut panel) = self.sftp_view_mut_for_connection(tab_id, connection_id, cx) {
            let err_msg = localized_error(&error);
            let mut failed_transfer = false;
            if let Some(task) = panel.state.queue.tasks.iter_mut().find(|t| t.id == op_id) {
                failed_transfer = true;
                task.status =
                    TransferStatus::Failed(TransferFailure::from_error(&error, err_msg.clone()));
                let file_name = task
                    .local_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| task.remote_path.clone());
                failed_notify = Some((file_name, err_msg.clone()));
            }
            if failed_transfer {
                panel.state.queue.active_transfers.remove(&op_id);
                panel.state.recompute_progress();
                panel.state.set_queue_error(err_msg);
            } else if let Some(target) = panel
                .state
                .remote_pane_of_connection(connection_id, main_conn)
                && let Some(pane) = panel.state.remote_pane_mut(target)
            {
                // 非传输失败：错误只落在目标列自己的内容区，主远端与并列列
                // 共用同一路径，互不覆盖加载状态。
                pane.loading = false;
                pane.error = Some(err_msg);
            }
        }
        self.sync_drop_overlay(tab_id, cx);
        if let Some((file_name, err_msg)) = failed_notify {
            send_desktop_notification(
                &rust_i18n::t!("sftp.handlers.notify_failed_title"),
                &format!("{file_name}: {err_msg}"),
            );
        }
        self.process_sftp_queue(tab_id, cx);
        true
    }
}
