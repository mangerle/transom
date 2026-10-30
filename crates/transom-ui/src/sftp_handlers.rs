//! SFTP 事件的域处理：目录回填、进度、完成/失败、属性与预览。
//!
//! 与 [`crate::event_handlers`] 的分工：本文件只处理 SFTP 传输相关事件，
//! 连接/认证与遥测事件在另一处；拆分让每个文件保持在工程规范的体量内。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::sftp::TransferStatus;
use crate::sftp::{
    RemoteDirCacheEntry, SftpPreviewResponse, SftpPropertiesModalState, format_size, parent_remote,
    remote_file_properties, send_desktop_notification,
};
use gpui_kit::*;
use transom_core::{SftpAppEvent, TransferDirection};

impl AppShell {
    /// SFTP 目录列表回填：主面板或并排远端列按连接归属分别落位。
    pub(crate) fn on_sftp_listing(
        &mut self,
        event: SftpAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let SftpAppEvent::SftpListing {
            tab_id,
            connection_id,
            path,
            entries,
        } = event
        else {
            return true;
        };
        let mut update_input = None;
        if let Some(tab) = self.workspace.tab_mut(tab_id) {
            let main_connection = tab.connection_id;
            if let Some(panel) = tab.sftp_mut() {
                if let Some(target) =
                    panel.remote_pane_of_connection(connection_id, main_connection)
                {
                    if let Some(pane) = panel.remote_pane_mut(target) {
                        log::debug!(
                            "远端目录回填，窗格: {tab_id}，目标: {target:?}，连接: {connection_id}，路径: {path}，条目数: {}",
                            entries.len()
                        );
                        pane.path = path.clone();
                        pane.dir_cache.insert(
                            path.clone(),
                            RemoteDirCacheEntry {
                                entries: entries.clone(),
                                cached_at: std::time::Instant::now(),
                            },
                        );
                        pane.entries = entries;
                        pane.loading = false;
                        pane.error = None;
                        let valid_paths: std::collections::BTreeSet<String> = pane
                            .entries
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect();
                        pane.sync_selection(&valid_paths);
                        update_input = Some((pane.path_input.clone(), path));
                    }
                } else {
                    // 列已关闭或事件标签不匹配：回填没有落点，留一条调试线索。
                    log::debug!(
                        "远端目录回填找不到目标列，窗格: {tab_id}，连接: {connection_id}，路径: {path}"
                    );
                }
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
    pub(crate) fn on_sftp_progress(&mut self, event: SftpAppEvent, cx: &mut Context<Self>) -> bool {
        let SftpAppEvent::SftpProgress {
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
            && let Some(task) = panel.queue.tasks.iter_mut().find(|t| t.id == op_id)
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
        if applied && self.workspace.active_tab == Some(tab_id) && !self.immersive {
            cx.notify();
        }
        false
    }

    /// SFTP 进入校验阶段：将任务状态更新为 `TransferStatus::Verifying`。
    pub(crate) fn on_sftp_verifying(
        &mut self,
        event: SftpAppEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let SftpAppEvent::SftpVerifying {
            tab_id,
            connection_id,
            op_id,
        } = event
        else {
            return true;
        };
        if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id)
            && let Some(task) = panel.queue.tasks.iter_mut().find(|t| t.id == op_id)
        {
            task.status = TransferStatus::Verifying;
            task.speed_bps = None;
            task.eta_secs = None;
            Self::set_sftp_transient_status(
                panel,
                tab_id,
                rust_i18n::t!("sftp.verifying").to_string(),
                cx,
            );
        }
        true
    }

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
        let mut refresh = None;
        let mut refresh_side = None;
        let mut refresh_local = false;
        let mut notify_info: Option<(TransferDirection, String, u64)> = None;
        let main_conn = self.workspace.tab(tab_id).and_then(|tab| tab.connection_id);
        if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
            let mut completed_transfer = false;
            if let Some(task) = panel.queue.tasks.iter_mut().find(|t| t.id == op_id) {
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
                // 上传落到并列远端列时刷新该列目录：完成路径原本只刷新主远端，
                // 挂载列看不到刚上传的文件。上传到主远端仍由下方统一刷新覆盖。
                if task.direction == TransferDirection::Upload
                    && let Some(target_conn) = task.target_connection_id
                    && Some(target_conn) != main_conn
                {
                    refresh_side = Some((target_conn, parent_remote(&task.remote_path)));
                }
                // 并列远端列的目录同步：任务记录的是远端根目录，完成后直接刷新该列根。
                if task.direction == TransferDirection::Sync
                    && let Some(target_conn) = task.target_connection_id
                    && Some(target_conn) != main_conn
                {
                    refresh_side = Some((target_conn, task.remote_path.clone()));
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
                panel.queue.active_transfers.remove(&op_id);
                panel.recompute_progress();
                if panel.queue.active_transfers.is_empty() {
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
                    panel.clear_queue_error();
                    refresh = Some(panel.remote.path.clone());
                }
            } else {
                // 非传输操作：按连接找到目标列并刷新它自己的当前目录；
                // 主远端与并列列不再各写一条完成分支。
                let pane_path = panel
                    .remote_pane_of_connection(connection_id, main_conn)
                    .and_then(|target| panel.remote_pane(target))
                    .map(|pane| pane.path.clone());
                if let Some(path) = pane_path {
                    refresh_side = Some((connection_id, path));
                }
                Self::set_sftp_transient_status(
                    panel,
                    tab_id,
                    rust_i18n::t!("sftp.operation_done").to_string(),
                    cx,
                );
            }
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

    /// SFTP 属性响应：迟到的响应（标签已关闭）直接忽略且不重绘。
    pub(crate) fn on_sftp_metadata(&mut self, event: SftpAppEvent, cx: &mut Context<Self>) -> bool {
        let SftpAppEvent::SftpMetadata {
            tab_id,
            connection_id,
            metadata,
            ..
        } = event
        else {
            return true;
        };
        if self.sftp_panel_mut(tab_id, connection_id).is_none() {
            // 标签页已关闭（或连接已切换）：忽略迟到的属性响应，不再弹出属性窗
            return false;
        }
        self.sftp_modals.properties = Some(SftpPropertiesModalState {
            scope: "远端".to_string(),
            properties: remote_file_properties(&metadata),
        });
        self.process_sftp_queue(tab_id, cx);
        true
    }

    /// SFTP 预览成功：交给预览响应处理统一落位。
    pub(crate) fn on_sftp_preview(&mut self, event: SftpAppEvent, cx: &mut Context<Self>) -> bool {
        let SftpAppEvent::SftpPreview {
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
        event: SftpAppEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let SftpAppEvent::SftpPreviewFailed {
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
    pub(crate) fn on_sftp_dir_compared(&mut self, event: SftpAppEvent) -> bool {
        let SftpAppEvent::SftpDirCompared {
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
        let main_conn = self.workspace.tab(tab_id).and_then(|tab| tab.connection_id);
        if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
            let err_msg = localized_error(&error);
            let mut failed_transfer = false;
            if let Some(task) = panel.queue.tasks.iter_mut().find(|t| t.id == op_id) {
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
                panel.queue.active_transfers.remove(&op_id);
                panel.recompute_progress();
                panel.set_queue_error(err_msg);
            } else if let Some(target) = panel.remote_pane_of_connection(connection_id, main_conn)
                && let Some(pane) = panel.remote_pane_mut(target)
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
