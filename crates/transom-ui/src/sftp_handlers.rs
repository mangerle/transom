//! SFTP 事件的域处理：目录回填、进度、完成/失败、属性与预览。
//!
//! 与 [`crate::event_handlers`] 的分工：本文件只处理 SFTP 传输相关事件，
//! 连接/认证与遥测事件在另一处；拆分让每个文件保持在工程规范的体量内。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::sftp::{
    SftpPreviewResponse, SftpPropertiesModalState, format_size, remote_file_properties,
    send_desktop_notification,
};
use gpui_kit::*;
use transom_core::{AppEvent, TransferDirection, TransferStatus};

impl AppShell {
    /// SFTP 目录列表回填：主面板或并排远端列按连接归属分别落位。
    pub(crate) fn on_sftp_listing(
        &mut self,
        event: AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let AppEvent::SftpListing {
            tab_id,
            connection_id,
            path,
            entries,
        } = event
        else {
            return true;
        };
        let mut update_input = None;
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && let Some(panel) = tab.sftp.as_mut()
        {
            if tab.connection_id == Some(connection_id) {
                panel.path = path.clone();
                panel.entries = entries;
                panel.loading = false;
                panel.error = None;
                let valid_paths = panel
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                panel.sync_remote_selection(&valid_paths);
                update_input = Some((panel.path_input.clone(), path));
            } else if let Some(side) = panel
                .side_remotes
                .iter_mut()
                .find(|s| s.connection_id == connection_id)
            {
                side.path = path.clone();
                side.entries = entries;
                side.loading = false;
                side.error = None;
                let valid_paths: std::collections::BTreeSet<String> = side
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                side.selection.retain(|p| valid_paths.contains(p));
                if let Some(sel) = &side.selected
                    && !valid_paths.contains(sel)
                {
                    side.selected = None;
                }
                update_input = Some((side.path_input.clone(), path));
            }
        }
        if let Some((input, path)) = update_input {
            input.update(cx, |state, cx| state.set_value(path, window, cx));
        }
        true
    }

    /// SFTP 进度：高频可覆盖样本，只更新数据；仅可见标签自行触发重绘。
    ///
    /// 返回 `false`：重绘由本方法条件决定（见内联注释），调用方不再统一 notify。
    pub(crate) fn on_sftp_progress(&mut self, event: AppEvent, cx: &mut Context<Self>) -> bool {
        let AppEvent::SftpProgress {
            tab_id,
            connection_id,
            op_id,
            transferred,
            total,
            speed_bps,
            eta_secs,
        } = event
        else {
            return true;
        };
        let mut applied = false;
        if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id)
            && let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id)
        {
            task.transferred = transferred;
            if total.is_some() {
                task.size = total;
            }
            task.speed_bps = speed_bps;
            task.eta_secs = eta_secs;
            panel.recompute_progress();
            applied = true;
        }
        // 进度事件高频到达：只有当前可见标签的传输队列在显示进度时
        // 才值得整帧重绘外壳；后台标签的数据照常更新，切回时自然呈现。
        if applied {
            // 浮层的速率与重绘按"窗格是否真的在屏上"判断，比只看活动窗格更宽：
            // 分屏里两个窗格可以同时显示各自的浮层。
            self.note_drop_overlay_progress(tab_id, cx);
        }
        if applied && self.active_tab == Some(tab_id) && !self.immersive {
            cx.notify();
        }
        false
    }

    /// SFTP 传输完成：落定任务状态、刷新受影响的目录并派发桌面通知。
    pub(crate) fn on_sftp_finished(&mut self, event: AppEvent, cx: &mut Context<Self>) -> bool {
        let AppEvent::SftpFinished {
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
        let mut refresh = None;
        let mut refresh_side = None;
        let mut refresh_local = false;
        let mut notify_info: Option<(TransferDirection, String, u64)> = None;
        if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
            let mut completed_transfer = false;
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                completed_transfer = true;
                task.status = TransferStatus::Completed;
                task.speed_bps = None;
                task.eta_secs = None;
                if let Some(bytes) = transferred {
                    task.transferred = bytes;
                }
                if task.direction == TransferDirection::Download
                    || task.direction == TransferDirection::Sync
                {
                    refresh_local = true;
                }
                if task.direction == TransferDirection::RemoteCopy
                    && let (Some(target_conn), Some(target_path)) =
                        (task.target_connection_id, &task.target_remote_path)
                {
                    let dir = target_path
                        .rsplit_once('/')
                        .map(|(d, _)| if d.is_empty() { "/" } else { d })
                        .unwrap_or("/");
                    refresh_side = Some((target_conn, dir.to_string()));
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
                notify_info = Some((task.direction, file_name, task.transferred));
            }
            if completed_transfer {
                panel.active_transfers.remove(&op_id);
                panel.recompute_progress();
                if panel.active_transfers.is_empty() && panel.active_op.is_none() {
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
                    Self::set_sftp_transient_status(panel, tab_id, message, cx);
                    panel.error = None;
                    refresh = Some(panel.path.clone());
                }
            } else if panel.active_op == Some(op_id) {
                panel.active_op = None;
                let base = match transferred {
                    Some(bytes) => format!(
                        "{} ({})",
                        rust_i18n::t!("sftp.transfer_done"),
                        format_size(bytes)
                    ),
                    None => rust_i18n::t!("sftp.operation_done").to_string(),
                };
                Self::set_sftp_transient_status(panel, tab_id, base, cx);
                refresh = Some(panel.path.clone());
            }
        }
        // 任务落定，浮层据此决定继续常驻还是进入 5 秒停留窗口。
        self.sync_drop_overlay(tab_id, cx);
        if let Some((dir, file_name, bytes)) = notify_info {
            let action = match dir {
                TransferDirection::Upload => "上传完成",
                TransferDirection::Download => "下载完成",
                TransferDirection::Sync => "同步完成",
                TransferDirection::RemoteCopy => "跨远端传输完成",
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

    /// SFTP 属性响应：迟到的响应（标签已关闭）直接忽略且不重绘。
    pub(crate) fn on_sftp_metadata(&mut self, event: AppEvent, cx: &mut Context<Self>) -> bool {
        let AppEvent::SftpMetadata {
            tab_id,
            connection_id,
            op_id,
            metadata,
        } = event
        else {
            return true;
        };
        let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) else {
            // 标签页已关闭（或连接已切换）：忽略迟到的属性响应，不再弹出属性窗
            return false;
        };
        if panel.active_op == Some(op_id) {
            panel.active_op = None;
        }
        self.sftp_modals.properties = Some(SftpPropertiesModalState {
            scope: "远端".to_string(),
            properties: remote_file_properties(&metadata),
        });
        self.process_sftp_queue(tab_id, cx);
        true
    }

    /// SFTP 预览成功：交给预览响应处理统一落位。
    pub(crate) fn on_sftp_preview(&mut self, event: AppEvent, cx: &mut Context<Self>) -> bool {
        let AppEvent::SftpPreview {
            tab_id,
            connection_id,
            op_id,
            bytes,
            truncated,
        } = event
        else {
            return true;
        };
        self.handle_sftp_preview(
            SftpPreviewResponse {
                tab_id,
                connection_id,
                op_id,
                result: Ok((bytes, truncated)),
            },
            cx,
        );
        true
    }

    /// SFTP 预览失败：与成功共用同一条响应处理路径。
    pub(crate) fn on_sftp_preview_failed(
        &mut self,
        event: AppEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let AppEvent::SftpPreviewFailed {
            tab_id,
            connection_id,
            op_id,
            error,
        } = event
        else {
            return true;
        };
        self.handle_sftp_preview(
            SftpPreviewResponse {
                tab_id,
                connection_id,
                op_id,
                result: Err(error),
            },
            cx,
        );
        true
    }

    /// 目录对比完成：仅在同步模态仍匹配该操作时回填结果。
    pub(crate) fn on_sftp_dir_compared(&mut self, event: AppEvent) -> bool {
        let AppEvent::SftpDirCompared {
            connection_id,
            op_id,
            comparison,
            ..
        } = event
        else {
            return true;
        };
        if let Some(modal) = self.sftp_modals.sync.as_mut()
            && modal.op_id == op_id
            && modal.connection_id == connection_id
        {
            modal.scanning = false;
            modal.skipped = comparison.skipped;
            modal.selected = comparison
                .entries
                .iter()
                .map(|entry| entry.sync_item().is_some())
                .collect();
            modal.entries = comparison.entries;
            modal.error = None;
        }
        true
    }

    /// SFTP 失败：同步模态失败自行重绘并返回；传输失败落定任务并通知。
    pub(crate) fn on_sftp_failed(&mut self, event: AppEvent, cx: &mut Context<Self>) -> bool {
        let AppEvent::SftpFailed {
            tab_id,
            connection_id,
            op_id,
            error,
        } = event
        else {
            return true;
        };
        if let Some(modal) = self.sftp_modals.sync.as_mut()
            && modal.op_id == op_id
            && modal.connection_id == connection_id
        {
            modal.scanning = false;
            modal.error = Some(localized_error(&error));
            cx.notify();
            return false;
        }
        let mut failed_notify: Option<(String, String)> = None;
        if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
            let err_msg = localized_error(&error);
            let mut failed_transfer = false;
            if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                failed_transfer = true;
                task.status = TransferStatus::Failed(err_msg.clone());
                let file_name = task
                    .local_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| task.remote_path.clone());
                failed_notify = Some((file_name, err_msg.clone()));
            }
            if failed_transfer {
                panel.active_transfers.remove(&op_id);
                panel.recompute_progress();
                panel.error = Some(err_msg);
            } else if panel.active_op == Some(op_id) || panel.active_op.is_none() {
                panel.active_op = None;
                panel.loading = false;
                panel.error = Some(err_msg);
            }
        }
        self.sync_drop_overlay(tab_id, cx);
        if let Some((file_name, err_msg)) = failed_notify {
            send_desktop_notification("Transom SFTP 传输失败", &format!("{file_name}: {err_msg}"));
        }
        self.process_sftp_queue(tab_id, cx);
        true
    }
}
