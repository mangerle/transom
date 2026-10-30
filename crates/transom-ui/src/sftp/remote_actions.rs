//! 远端列的动作辅助入口：按目标列解析连接、上传与跨列复制。
//!
//! 设计原理（Why）：
//! - 实现初衷：并列远端列此前只实现了部分动作，主远端的导航、目录操作与
//!   上下文菜单各写一套；统一状态后，动作直接按目标列寻址，不再默认落到
//!   `panel.remote`；
//! - 核心优势：文件操作、弹窗与导航都只有一份连接化实现，调用方只传
//!   `SftpRemotePaneRef`，新增列能力不需要再补一套并列列分支；
//! - 代价与局限：主远端与本地栏共用弹窗入口时仍需区分"远端目标/本地目标"，
//!   这属于本地与远端两种语义，不再是主列与并列列的重复实现。
//!
//! 导航与过滤动作在 `remote_nav`，文件操作实现分散在各自的业务文件中。

use crate::app::AppShell;
use crate::sftp::side_remote::SftpRemoteCopyParams;
use crate::sftp::{SftpRemotePaneRef, SftpRemotePaneState, parent_remote};
use gpui_kit::*;
use transom_core::{ConnectionId, SftpCommand, SftpOperation, TabId, TransferId};

/// 一次远端列动作的操作域：目标列、实际连接与弹窗应携带的连接语义。
///
/// `side_connection` 为 `None` 表示主远端（弹窗提交时沿用标签页主连接），
/// `Some` 表示并列列。集中在这里解析，避免每个动作各写一遍查找口径。
pub(crate) struct RemotePaneScope<'a> {
    pub(crate) pane: &'a SftpRemotePaneState,
    pub(crate) connection_id: ConnectionId,
    pub(crate) side_connection: Option<ConnectionId>,
}

impl AppShell {
    /// 解析目标远端列、实际连接与弹窗连接语义。
    pub(crate) fn remote_pane_scope(
        &self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
    ) -> Option<RemotePaneScope<'_>> {
        let tab = self.workspace.tab(tab_id)?;
        let panel = tab.sftp()?;
        let pane = panel.remote_pane(target)?;
        let connection_id = panel.remote_connection(target, tab.connection_id)?;
        let side_connection = match target {
            SftpRemotePaneRef::Main => None,
            SftpRemotePaneRef::Side(_) => Some(connection_id),
        };
        Some(RemotePaneScope {
            pane,
            connection_id,
            side_connection,
        })
    }
    /// 向指定连接下发 SFTP 非传输操作。
    ///
    /// 主远端与并列远端共用同一条投递路径：连接先反解为目标列，面板消息与
    /// 目标列错误都在这里统一清理，完成事件再按连接刷新对应列。
    pub(crate) fn send_sftp_op_for_connection(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        op: SftpOperation,
        cx: &mut Context<Self>,
    ) {
        let target = self.workspace.tab(tab_id).and_then(|tab| {
            tab.sftp()
                .and_then(|panel| panel.remote_pane_of_connection(connection_id, tab.connection_id))
        });
        let Some(target) = target else {
            return;
        };
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.clear_status();
            if let Some(pane) = panel.remote_pane_mut(target) {
                pane.error = None;
            }
        }
        self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id: TransferId::new(),
            op,
        });
        cx.notify();
    }

    /// 目标列对应的实际连接 ID。
    pub(crate) fn remote_pane_connection(
        &self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
    ) -> Option<ConnectionId> {
        self.remote_pane_scope(tab_id, target)
            .map(|scope| scope.connection_id)
    }

    /// 把目标远端列设为活动栏并重绘。
    pub(crate) fn activate_remote_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.active_pane = target.into();
        }
        cx.notify();
    }

    /// 右键菜单定位条目：已多选时只改主项，否则清空旧选择只选当前项。
    pub(crate) fn select_remote_entry_for_menu(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(pane) = panel.remote_pane_mut(target)
        {
            if pane.selection.contains(&path) {
                pane.set_primary(path);
            } else {
                pane.select(path);
            }
            panel.active_pane = target.into();
        }
        cx.notify();
    }

    /// 在目标列当前目录直接解压（不弹命名框）。
    pub(crate) fn sftp_extract_here_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        archive_path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.remote_pane_connection(tab_id, target) else {
            return;
        };
        let dest_dir = parent_remote(&archive_path);
        self.send_sftp_op_for_connection(
            tab_id,
            connection_id,
            SftpOperation::Extract {
                archive_path,
                dest_dir,
            },
            cx,
        );
    }

    /// 目标列的编辑器入口。
    pub(crate) fn sftp_edit_remote_file_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.remote_pane_connection(tab_id, target) else {
            return;
        };
        self.sftp_edit_remote_file_on(tab_id, connection_id, path, window, cx);
    }

    /// 目标列的上传入口：把本地列选中项上传到该列当前目录。
    pub(crate) fn sftp_upload_local_selection_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some((paths, target_dir, connection_id)) = self.workspace.tab(tab_id).and_then(|tab| {
            let panel = tab.sftp()?;
            let pane = panel.remote_pane(target)?;
            let connection_id = panel.remote_connection(target, tab.connection_id)?;
            Some((
                panel.selected_local_paths(),
                pane.path.clone(),
                connection_id,
            ))
        }) else {
            return;
        };
        if paths.is_empty() {
            self.set_sftp_pane_error(
                tab_id,
                target,
                rust_i18n::t!("sftp.select_file").to_string(),
                cx,
            );
            return;
        }
        if let Err(message) =
            self.enqueue_uploads_to(tab_id, &paths, &target_dir, connection_id, cx)
        {
            self.set_sftp_pane_error(tab_id, target, message, cx);
        }
    }

    /// 目标列的"复制到主远端"入口（并列列专属菜单项）。
    pub(crate) fn sftp_copy_remote_entry_to_main(
        &mut self,
        tab_id: TabId,
        source: SftpRemotePaneRef,
        path: String,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(source_connection_id) = self.remote_pane_connection(tab_id, source) else {
            return;
        };
        let Some((target_connection_id, target_dir)) = self.workspace.tab(tab_id).and_then(|tab| {
            let connection_id = tab.connection_id?;
            let target_dir = tab.sftp()?.remote.path.clone();
            Some((connection_id, target_dir))
        }) else {
            return;
        };
        let fname = path.rsplit('/').next().unwrap_or(&path);
        let target_path = format!("{}/{}", target_dir.trim_end_matches('/'), fname);
        self.sftp_enqueue_remote_transfer(
            tab_id,
            SftpRemoteCopyParams {
                source_connection_id,
                source_path: path,
                target_connection_id,
                target_path,
                is_dir,
            },
            cx,
        );
    }

    /// 目标列进入加载态：主远端与并列列都清掉本列旧错误并置加载中。
    ///
    /// 错误与加载态自本列统一后，不再区分面板状态栏与列内两套存储。
    pub(crate) fn begin_sftp_pane_load(&mut self, tab_id: TabId, target: SftpRemotePaneRef) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(pane) = panel.remote_pane_mut(target)
        {
            pane.loading = true;
            pane.error = None;
        }
    }

    /// 把错误落到目标列的内容区。
    pub(crate) fn set_sftp_pane_error(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(pane) = panel.remote_pane_mut(target)
        {
            pane.error = Some(message);
        }
        cx.notify();
    }
}
