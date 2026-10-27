//! SFTP 目录对比与增量同步：打开对比弹窗、勾选差异项并创建同步任务。

use super::SftpSyncModalState;
use crate::app::AppShell;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::*;
use transom_core::{
    SftpCommand, SftpOperation, SyncDirection, TabId, TransferFilter, TransferId, TransferTask,
};

impl AppShell {
    /// 以当前双栏路径为根目录发起异步扫描对比。
    pub(crate) fn sftp_open_sync_modal(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let filter_text = self.settings.sftp.transfer_filter.clone();
        let Some(tab) = self.workspace.tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp() else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let local_root = panel.local_path.clone();
        let remote_root = panel.path.clone();
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
            modal.error = Some("请至少勾选一个可同步项".to_string());
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
        if let Some(panel) = self
            .workspace
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.queue.push(task);
            panel.status = Some(format!(
                "已加入目录同步任务（上传 {upload_count} 项，下载 {download_count} 项）"
            ));
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }
}
