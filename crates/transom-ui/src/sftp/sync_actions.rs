//! SFTP 目录对比与增量同步：打开对比弹窗、勾选差异项并创建同步任务。

use super::SftpSyncModalState;
use crate::app::AppShell;
use crate::sftp::{SftpRemotePaneRef, TransferTask};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::*;
use transom_core::{SftpCommand, SftpOperation, SyncDirection, TabId, TransferFilter, TransferId};

impl AppShell {
    /// 以当前双栏路径为根目录发起异步扫描对比。
    pub(crate) fn sftp_open_sync_modal(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.sftp_open_sync_modal_pane(tab_id, SftpRemotePaneRef::Main, cx);
    }

    /// 以目标远端列与本地当前目录为根发起目录对比。
    ///
    /// 并列远端列复用同一条对比与同步通道：扫描请求、弹窗状态与同步任务都
    /// 携带目标连接，不会回落到标签页主远端。
    pub(crate) fn sftp_open_sync_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let filter_text = self.settings.sftp.transfer_filter.clone();
        let Some((connection_id, local_root, remote_root)) =
            self.workspace.tab(tab_id).and_then(|tab| {
                let panel = tab.sftp()?;
                let pane = panel.remote_pane(target)?;
                let connection_id = panel.remote_connection(target, tab.connection_id)?;
                Some((connection_id, panel.local.path.clone(), pane.path.clone()))
            })
        else {
            return;
        };
        let op_id = TransferId::new();
        self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::CompareDirs {
                local_path: local_root.clone(),
                remote_path: remote_root.clone(),
                filter: TransferFilter::from_text(&filter_text),
            },
        });
        self.sftp_modals.sync = Some(SftpSyncModalState {
            tab_id,
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
        });
        cx.notify();
    }

    pub(crate) fn sftp_toggle_sync_entry(
        &mut self,
        index: usize,
        checked: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.sftp_modals.sync.as_mut() else {
            return;
        };
        let Some(entry) = modal.entries.get(index) else {
            return;
        };
        if checked && entry.sync_item().is_none() {
            return;
        }
        if let Some(slot) = modal.selected.get_mut(index) {
            *slot = checked;
            cx.notify();
        }
    }

    /// 全选/全不选可同步项；类型冲突始终保持不可选。
    pub(crate) fn sftp_select_all_sync(&mut self, selected: bool, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_modals.sync.as_mut() else {
            return;
        };
        for (index, entry) in modal.entries.iter().enumerate() {
            if let Some(slot) = modal.selected.get_mut(index) {
                *slot = selected && entry.sync_item().is_some();
            }
        }
        cx.notify();
    }

    /// 将勾选的差异项打包成一个同步任务入队并启动队列。
    pub(crate) fn start_sftp_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(mut modal) = self.sftp_modals.sync.take() else {
            return;
        };
        if modal.scanning || modal.error.is_some() {
            self.sftp_modals.sync = Some(modal);
            return;
        }
        let tab_id = modal.tab_id;
        let mut upload_count = 0u32;
        let mut download_count = 0u32;
        let mut total_size = 0u64;
        let items = modal
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if !modal.selected.get(index).copied().unwrap_or(false) {
                    return None;
                }
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
            modal.error = Some(rust_i18n::t!("sftp.nav.sync_no_selection").to_string());
            self.sftp_modals.sync = Some(modal);
            cx.notify();
            return;
        }

        let mut task = TransferTask::new_sync(
            modal.local_root.clone(),
            modal.remote_root.clone(),
            items,
            Some(total_size),
        );
        task.verify = verify;
        task.verify_mode = self.settings.sftp.verify_mode;
        task.checksum_algorithm = self.settings.sftp.checksum_algorithm;
        task.target_connection_id = Some(modal.connection_id);
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.queue.tasks.push(task);
            panel.set_status(
                rust_i18n::t!(
                    "sftp.nav.sync_queued",
                    upload = upload_count,
                    download = download_count
                )
                .to_string(),
            );
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }
}
