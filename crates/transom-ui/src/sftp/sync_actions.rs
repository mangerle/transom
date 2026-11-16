//! SFTP 目录对比与增量同步：打开对比弹窗、勾选差异项并创建同步任务。

use super::SftpSyncModalState;
use super::entity::SftpPanelView;
use crate::app::app_settings;
use crate::app::{AppShell, AppState};
use crate::sftp::{SftpRemotePaneRef, TransferTask};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::*;
use transom_core::{
    SftpCommand, SftpOperation, SyncDirection, TabId, TransferFilter, TransferId, VerifyOptions,
};

impl SftpPanelView {
    /// 以当前双栏路径为根目录发起异步扫描对比。
    pub(crate) fn open_sync_modal(&mut self, cx: &mut Context<Self>) {
        self.open_sync_modal_pane(SftpRemotePaneRef::Main, cx);
    }

    /// 以目标远端列与本地当前目录为根发起目录对比。
    pub(crate) fn open_sync_modal_pane(
        &mut self,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(main_connection) = self.state.main_connection else {
            return;
        };
        let Some(pane) = self.state.remote_pane(target) else {
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            return;
        };
        let local_root = self.state.local.path.clone();
        let remote_root = pane.path.clone();
        let op_id = TransferId::new();
        let filter_text = app_settings(cx).sftp.transfer_filter.clone();
        cx.emit(crate::sftp::SftpPanelEvent::SendCommand(
            SftpCommand::Sftp {
                tab_id: self.tab_id,
                connection_id,
                op_id,
                op: SftpOperation::CompareDirs {
                    local_path: local_root.clone(),
                    remote_path: remote_root.clone(),
                    filter: TransferFilter::from_text(&filter_text),
                },
            },
        ));
        cx.emit(crate::sftp::SftpPanelEvent::OpenSyncModal(Box::new(
            SftpSyncModalState {
                tab_id: self.tab_id,
                connection_id,
                op_id,
                local_root,
                remote_root,
                scanning: true,
                entries: Vec::new(),
                selected: Vec::new(),
                skipped: 0,
                error: None,
                scroll_handle: VirtualListScrollHandle::new(),
            },
        )));
        cx.notify();
    }
}

pub(crate) fn sftp_open_sync_modal(
    app_state: &mut AppState,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    if let Some(view) = app_state.sftp_view(tab_id) {
        view.update(cx, |view, cx| view.open_sync_modal(cx));
    }
}

/// 将勾选的差异项打包成一个同步任务入队并启动队列。返回是否成功入队。
pub(crate) fn execute_sftp_sync(
    app_state: &mut AppState,
    tab_id: TabId,
    connection_id: Option<transom_core::ConnectionId>,
    local_root: &std::path::Path,
    remote_root: &str,
    entries: &[transom_core::DirDiffEntry],
    cx: &mut Context<AppShell>,
) -> bool {
    let verify = app_settings(cx).sftp.verify_after_transfer;
    let mut upload_count = 0u32;
    let mut download_count = 0u32;
    let mut total_size = 0u64;
    let items = entries
        .iter()
        .filter_map(|entry| {
            let item = entry.sync_item()?;
            match item.direction {
                SyncDirection::Upload => upload_count = upload_count.saturating_add(1),
                SyncDirection::Download => download_count = download_count.saturating_add(1),
            }
            if !item.is_dir {
                total_size = total_size.saturating_add(item.size);
            }
            Some(item)
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        if let Some(modal) = app_state.ui.modals.sftp.sync.as_ref() {
            modal.update(cx, |m, cx| {
                m.state.error = Some(rust_i18n::t!("sftp.nav.sync_no_selection").to_string());
                cx.notify();
            });
        }
        return false;
    }

    let mut task = TransferTask::new_sync(
        local_root.to_path_buf(),
        remote_root.to_string(),
        items,
        Some(total_size),
    );
    task.verify = VerifyOptions {
        enabled: verify,
        mode: app_settings(cx).sftp.verify_mode,
        algorithm: app_settings(cx).sftp.checksum_algorithm,
    };
    task.target_connection_id = connection_id;
    if let Some(mut panel) = crate::sftp::sftp_view_mut(&app_state.workspace, tab_id, cx) {
        panel.state.queue.tasks.push(task);
        panel.state.set_status(
            rust_i18n::t!(
                "sftp.nav.sync_queued",
                upload = upload_count,
                download = download_count
            )
            .to_string(),
        );
    }
    cx.notify();
    if let Some(view) = app_state.sftp_view(tab_id) {
        view.update(cx, |view, cx| view.process_queue(tab_id, cx));
    }
    true
}
