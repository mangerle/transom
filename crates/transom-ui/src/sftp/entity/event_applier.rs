//! SFTP 面板视图实体响应底层通信事件的方法实现。
//!
//! 设计原理（Why）：底层控制器派发回来的目录列举、传输进度、校验中与重试
//! 状态变更均属于面板实体的私有状态管理范畴。将这些逻辑收拢于实体内部，
//! 避免外壳事件处理器越权穿透修改面板内部字段，保证单一事实源。

use super::SftpPanelView;
use crate::sftp::{RemoteDirCacheEntry, TransferStatus};
use gpui_kit::*;
use std::collections::BTreeSet;
use std::time::Instant;
use transom_core::{ConnectionId, RemoteFile, TransferId};

impl SftpPanelView {
    /// 应用远端目录列举结果，回填到对应目标列并同步路径输入框。
    pub(crate) fn apply_listing(
        &mut self,
        connection_id: ConnectionId,
        path: String,
        entries: Vec<RemoteFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(main_connection) = self.state.main_connection else {
            return;
        };
        let Some(target) = self
            .state
            .remote_pane_of_connection(connection_id, Some(main_connection))
        else {
            log::debug!(
                "远端目录回填找不到目标列，窗格: {}，连接: {connection_id}，路径: {path}",
                self.tab_id
            );
            return;
        };
        let Some(pane) = self.state.remote_pane_mut(target) else {
            return;
        };
        log::debug!(
            "远端目录回填，窗格: {}，目标: {target:?}，连接: {connection_id}，路径: {path}，条目数: {}",
            self.tab_id,
            entries.len()
        );
        pane.path = path.clone();
        pane.dir_cache.insert(
            path.clone(),
            RemoteDirCacheEntry {
                entries: entries.clone(),
                cached_at: Instant::now(),
            },
        );
        pane.entries = entries;
        pane.loading = false;
        pane.error = None;
        let valid_paths: BTreeSet<String> = pane
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        pane.sync_selection(&valid_paths);
        pane.path_input
            .update(cx, |state, cx| state.set_value(path, window, cx));
        cx.notify();
    }

    /// 应用高频传输进度样本更新。
    pub(crate) fn apply_progress(
        &mut self,
        op_id: TransferId,
        transferred: u64,
        total: Option<u64>,
        speed_bps: Option<u64>,
        eta_secs: Option<u64>,
        _cx: &mut Context<Self>,
    ) -> bool {
        let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id) else {
            return false;
        };
        task.transferred = transferred;
        if total.is_some() {
            task.size = total;
        }
        task.speed_bps = speed_bps;
        task.eta_secs = eta_secs;
        self.state.recompute_progress();
        true
    }

    /// 进入数据校验状态，标记对应传输任务并呈现暂态提示。
    pub(crate) fn apply_verifying(&mut self, op_id: TransferId, cx: &mut Context<Self>) {
        if let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id) {
            task.status = TransferStatus::Verifying;
            task.speed_bps = None;
            task.eta_secs = None;
        }
        self.set_transient_status(rust_i18n::t!("sftp.verifying").to_string(), cx);
    }

    /// 校验不一致自动重传，任务进度归零并呈现暂态提示。
    pub(crate) fn apply_retrying(
        &mut self,
        op_id: TransferId,
        reason: &str,
        cx: &mut Context<Self>,
    ) {
        log::warn!("传输校验不一致，开始从头覆盖重传，任务: {op_id}，原因: {reason}");
        if let Some(task) = self.state.queue.tasks.iter_mut().find(|t| t.id == op_id) {
            task.plan_restart();
            task.status = TransferStatus::Transferring;
            task.speed_bps = None;
            task.eta_secs = None;
            self.state.recompute_progress();
        }
        self.set_transient_status(rust_i18n::t!("sftp.retrying").to_string(), cx);
    }
}
