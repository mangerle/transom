//! SFTP 并列远端的会话状态与动作：挂载、折叠、跨远端剪贴板与拖放。
//!
//! 视图渲染在 `side_remote_view`；本模块只持有数据与操作，不构建元素。
//!
//! 设计原理（Why）：
//! - 实现初衷：支持在单个终端标签的 SFTP 视图中挂载多个远端会话（本地 + 远端 A + 远端 B...），
//!   免去在多个标签页之间反复来回切换，直观实现同屏比对与跨主机流式复制；
//! - 核心优势：
//!   1. 各列（本地列与并列远端列）均支持独立一键收起折叠，最大化屏幕有效阅读空间；
//!   2. 支持跨列拖拽（Drag & Drop）、Ctrl+C / Ctrl+V 剪贴板与右键菜单三重交互闭环；
//!   3. 挂载器支持复用当前已建立连接的 SSH 会话（秒级挂载）或快速连接已保存会话；
//! - 代价与局限：并列多列受屏幕实际宽度影响，单列过窄时由横向自适应和列折叠共同吸收。

mod clipboard;
mod types;

pub(crate) use self::types::{
    SftpActivePane, SftpClipboardData, SftpClipboardSource, SftpDragItem, SftpDragPreview,
    SftpRemoteCopyParams, SftpSideRemoteState,
};

use crate::app::AppShell;
use crate::sftp::{TransferMode, TransferTask};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{ConnectionId, TabId, TransferDirection};

impl AppShell {
    /// 挂载并列远端会话到当前 SFTP 面板。
    pub(crate) fn sftp_mount_side_remote(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        title: String,
        initial_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        // 避免重复挂载相同连接
        if panel
            .side_remotes
            .iter()
            .any(|s| s.connection_id == connection_id)
        {
            panel.set_status(format!("该远端会话已挂载: {title}"));
            cx.notify();
            return;
        }

        let side_state =
            SftpSideRemoteState::new(connection_id, title, initial_path.clone(), window, cx);
        panel.side_remotes.push(side_state);
        panel.mount_menu_open = false;
        self.request_sftp_list_for_connection(tab_id, connection_id, initial_path, cx);
        cx.notify();
    }

    /// 关闭并列远端列。
    pub(crate) fn sftp_close_side_remote(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && index < panel.side_remotes.len()
        {
            panel.side_remotes.remove(index);
            if panel.active_pane == SftpActivePane::SideRemote(index) {
                panel.active_pane = SftpActivePane::Remote;
            }
            cx.notify();
        }
    }

    /// 切换并列远端列的展开与折叠。
    pub(crate) fn sftp_toggle_side_remote_collapse(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(side) = panel.side_remotes.get_mut(index)
        {
            side.collapsed = !side.collapsed;
            cx.notify();
        }
    }

    /// 切换本地列的展开与折叠。
    pub(crate) fn sftp_toggle_local_collapse(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.local.collapsed = !panel.local.collapsed;
            cx.notify();
        }
    }

    /// 切换主远端列的展开与折叠。
    pub(crate) fn sftp_toggle_remote_collapse(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.remote.collapsed = !panel.remote.collapsed;
            cx.notify();
        }
    }

    /// 跨远端互传任务入队。
    pub(crate) fn sftp_enqueue_remote_transfer(
        &mut self,
        tab_id: TabId,
        params: SftpRemoteCopyParams,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let src_path = params.source_path.clone();
        let tgt_path = params.target_path.clone();
        let task = TransferTask::new_remote_copy(
            params.source_connection_id,
            params.source_path,
            params.target_connection_id,
            params.target_path,
            None,
            params.is_dir,
        );
        panel.queue.tasks.push(task);
        panel.set_status(format!("已添加跨远端传输任务: {src_path} → {tgt_path}"));
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 将单个下载路径加入传输队列。
    pub(crate) fn sftp_enqueue_download_path(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.sftp_enqueue_download_path_with_conn(tab_id, None, remote_path, local_path, cx);
    }

    /// 将单个下载路径加入传输队列，支持显式指定源远端连接 ID（用于并列远端与本地面板互通）。
    pub(crate) fn sftp_enqueue_download_path_with_conn(
        &mut self,
        tab_id: TabId,
        source_connection_id: Option<ConnectionId>,
        remote_path: String,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let mode = if panel.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut task = TransferTask::new(
            TransferDirection::Download,
            local_path,
            remote_path.clone(),
            None,
            mode,
        );
        task.source_connection_id = source_connection_id;
        task.verify = verify;
        panel.queue.tasks.push(task);
        panel.set_status(format!("已加入下载: {remote_path}"));
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 将单个上传路径加入传输队列。
    pub(crate) fn sftp_enqueue_upload_path(
        &mut self,
        tab_id: TabId,
        local_path: PathBuf,
        target_remote: String,
        cx: &mut Context<Self>,
    ) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let mode = if panel.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut task = TransferTask::new(
            TransferDirection::Upload,
            local_path.clone(),
            target_remote.clone(),
            None,
            mode,
        );
        task.verify = verify;
        panel.queue.tasks.push(task);
        panel.set_status(format!("已加入上传: {}", local_path.display()));
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 辅助方法：直接按 tab_id 获取 sftp 面板可变借用。
    pub(crate) fn sftp_panel_mut_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<&mut super::SftpPanelState> {
        self.workspace
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp_mut())
    }

    /// 请求特定连接的远端目录列表（用于并列远端）。
    pub(crate) fn request_sftp_list_for_connection(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        path: String,
        _cx: &mut Context<Self>,
    ) {
        use transom_core::{SftpCommand, SftpOperation, TransferId};
        let op_id = TransferId::new();
        let cmd = SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::List { path },
        };
        self.send_command(cmd);
    }
}
