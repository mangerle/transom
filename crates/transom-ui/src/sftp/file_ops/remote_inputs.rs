//! 远端输入类弹窗：归档压缩、解压到新文件夹与创建软链接。
//!
//! 设计原理（Why）：这些弹窗都只需"目标连接 + 当前目录 + 提交载荷"，从
//! `file_ops` 主文件拆出后，归档/解压/软链接的文案与装配集中在一处，
//! 主文件只保留增删改查与删除确认。

use crate::app::AppShell;
use crate::sftp::SftpRemotePaneRef;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    pub(crate) fn sftp_open_archive_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        source_paths: Vec<String>,
        default_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.open_archive_modal(target, source_paths, default_name, window, cx)
            });
        }
    }

    pub(crate) fn sftp_open_extract_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        archive_path: String,
        default_dir_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.open_extract_modal(target, archive_path, default_dir_name, window, cx)
            });
        }
    }

    pub(crate) fn sftp_open_symlink_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.open_symlink_modal(target, window, cx));
        }
    }
}
