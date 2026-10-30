//! 远端输入类弹窗：归档压缩、解压到新文件夹与创建软链接。
//!
//! 设计原理（Why）：这些弹窗都只需"目标连接 + 当前目录 + 提交载荷"，从
//! `file_ops` 主文件拆出后，归档/解压/软链接的文案与装配集中在一处，
//! 主文件只保留增删改查与删除确认。

use crate::app::AppShell;
use crate::sftp::{SftpInputModalParams, SftpInputTarget, SftpRemotePaneRef};
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 打开目标远端列的归档压缩弹窗：并列列把连接写进弹窗，提交按连接路由。
    pub(crate) fn sftp_open_archive_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        source_paths: Vec<String>,
        default_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((side_connection, parent_dir)) = self
            .remote_pane_scope(tab_id, target)
            .map(|scope| (scope.side_connection, scope.pane.path.clone()))
        else {
            return;
        };
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: side_connection,
                title: rust_i18n::t!("sftp.file_ops.archive_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.archive_prompt", count = source_paths.len())
                    .to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.archive_placeholder").to_string(),
                value: default_name,
                target: SftpInputTarget::RemoteArchive {
                    source_paths,
                    parent_dir,
                },
            },
            window,
            cx,
        );
    }

    /// 打开目标远端列的解压到新文件夹弹窗。
    pub(crate) fn sftp_open_extract_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        archive_path: String,
        default_dir_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((side_connection, parent_dir)) = self
            .remote_pane_scope(tab_id, target)
            .map(|scope| (scope.side_connection, scope.pane.path.clone()))
        else {
            return;
        };
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: side_connection,
                title: rust_i18n::t!("sftp.file_ops.extract_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.extract_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.extract_placeholder").to_string(),
                value: default_dir_name,
                target: SftpInputTarget::RemoteExtract {
                    archive_path,
                    parent_dir,
                },
            },
            window,
            cx,
        );
    }

    /// 目标远端列的「创建软链接」输入弹窗。
    pub(crate) fn sftp_open_symlink_modal_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((side_connection, entry)) =
            self.remote_pane_scope(tab_id, target).and_then(|scope| {
                let selected = scope.pane.selected.clone()?;
                let entry = scope
                    .pane
                    .entries
                    .iter()
                    .find(|entry| entry.path == selected)?
                    .clone();
                Some((scope.side_connection, entry))
            })
        else {
            return;
        };
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: side_connection,
                title: rust_i18n::t!("sftp.file_ops.symlink_remote_title").to_string(),
                prompt: rust_i18n::t!(
                    "sftp.file_ops.symlink_remote_prompt",
                    path = entry.path.as_str()
                )
                .to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.symlink_name_placeholder").to_string(),
                value: format!("{}.link", entry.name),
                target: SftpInputTarget::RemoteSymlink { target: entry.path },
            },
            window,
            cx,
        );
    }
}
