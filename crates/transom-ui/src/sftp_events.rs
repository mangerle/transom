//! SFTP 列表、进度、校验、元数据与预览事件的处理。

use crate::app::AppShell;
use crate::sftp::TransferStatus;
use crate::sftp::{
    RemoteDirCacheEntry, SftpPreviewResponse, SftpPropertiesModalState, remote_file_properties,
};
use gpui_kit::*;
use transom_core::SftpAppEvent;

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
        if let Some(main_connection) = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id)
            && let Some(mut panel) = self.sftp_view_mut_for_connection(tab_id, connection_id, cx)
        {
            if let Some(target) = panel
                .state
                .remote_pane_of_connection(connection_id, Some(main_connection))
            {
                if let Some(pane) = panel.state.remote_pane_mut(target) {
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
        if let Some(mut panel) = self.sftp_view_mut_for_connection(tab_id, connection_id, cx)
            && let Some(task) = panel.state.queue.tasks.iter_mut().find(|t| t.id == op_id)
        {
            task.transferred = transferred;
            if total.is_some() {
                task.size = total;
            }
            task.speed_bps = speed_bps;
            task.eta_secs = eta_secs;
            panel.state.recompute_progress();
            applied = true;
        }
        // 进度事件高频到达：只有当前可见标签的传输队列在显示进度时
        // 才值得整帧重绘外壳；后台标签的数据照常更新，切回时自然呈现。
        if applied {
            // 浮层的速率与重绘按"窗格是否真的在屏上"判断，比只看活动窗格更宽：
            // 分屏里两个窗格可以同时显示各自的浮层。
            self.note_drop_overlay_progress(tab_id, cx);
        }
        if applied && self.state.workspace.active_tab == Some(tab_id) && !self.state.immersive {
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
        if let Some(mut panel) = self.sftp_view_mut_for_connection(tab_id, connection_id, cx)
            && let Some(task) = panel.state.queue.tasks.iter_mut().find(|t| t.id == op_id)
        {
            task.status = TransferStatus::Verifying;
            task.speed_bps = None;
            task.eta_secs = None;
        }
        self.set_sftp_transient_status(tab_id, rust_i18n::t!("sftp.verifying").to_string(), cx);
        true
    }

    /// SFTP 校验不一致后的自动恢复：任务回到传输中，进度归零。
    ///
    /// 设计原理（Why）：界面在这里只反映“从头重传”这一事实，不自行决定是否
    /// 重传——控制层已经把所有目标端内容丢掉重新写入，界面若继续显示“校验中”
    /// 或保留旧进度，会让用户看到一条与事实相反的队列。
    pub(crate) fn on_sftp_retrying(&mut self, event: SftpAppEvent, cx: &mut Context<Self>) -> bool {
        let SftpAppEvent::SftpRetrying {
            tab_id,
            connection_id,
            op_id,
            reason,
        } = event
        else {
            return true;
        };
        log::warn!("传输校验不一致，开始从头覆盖重传，任务: {op_id}，原因: {reason}");
        if let Some(mut panel) = self.sftp_view_mut_for_connection(tab_id, connection_id, cx)
            && let Some(task) = panel.state.queue.tasks.iter_mut().find(|t| t.id == op_id)
        {
            task.plan_restart();
            task.status = TransferStatus::Transferring;
            task.speed_bps = None;
            task.eta_secs = None;
            panel.state.recompute_progress();
        }
        self.set_sftp_transient_status(tab_id, rust_i18n::t!("sftp.retrying").to_string(), cx);
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
        if self
            .sftp_view_mut_for_connection(tab_id, connection_id, cx)
            .is_none()
        {
            // 标签页已关闭（或连接已切换）：忽略迟到的属性响应，不再弹出属性窗
            return false;
        }
        self.state.sftp_modals.properties = Some(SftpPropertiesModalState {
            scope: rust_i18n::t!("sftp.modals.scope_remote").to_string(),
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
        if let Some(modal) = self.state.sftp_modals.sync.as_mut()
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
}
