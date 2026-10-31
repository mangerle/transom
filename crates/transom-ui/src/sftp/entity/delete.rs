//! 实体侧的删除入口：只收集本面板的选中项并打开确认弹窗。
//!
//! 破坏性操作必须先经一次显式确认；确认后的实际删除仍由对话框提交路径
//! 回到外壳执行，因为那一步需要投递命令与后台 IO。

use super::*;
use crate::sftp::{SftpConfirmModalState, SftpPendingDelete, SftpRemotePaneRef};
use transom_core::RemoteDeleteTarget;

impl SftpPanelView {
    /// Delete 快捷键：删除当前活动栏选中的项目，先弹确认。
    pub(crate) fn delete_selected(&mut self, cx: &mut Context<Self>) {
        match self.state.active_pane {
            crate::sftp::SftpActivePane::Local => self.confirm_local_delete(cx),
            crate::sftp::SftpActivePane::Remote => {
                self.confirm_remote_delete(SftpRemotePaneRef::Main, cx)
            }
            crate::sftp::SftpActivePane::SideRemote(connection_id) => {
                self.confirm_remote_delete(SftpRemotePaneRef::Side(connection_id), cx)
            }
        }
    }

    fn confirm_local_delete(&mut self, cx: &mut Context<Self>) {
        let paths = self.state.selected_local_paths();
        if paths.is_empty() {
            self.state
                .set_local_error(rust_i18n::t!("sftp.local_async.select_first").to_string());
            cx.notify();
            return;
        }
        let tab_id = self.tab_id;
        let count = paths.len();
        let app = self.app.clone();
        with_shell_parts(&app, cx, |shell, _| {
            shell.state.sftp_modals.confirm = Some(SftpConfirmModalState {
                action: SftpPendingDelete::Local { tab_id, paths },
                title: rust_i18n::t!("sftp.local_async.delete_title").to_string(),
                description: rust_i18n::t!("sftp.local_async.delete_body", count = count)
                    .to_string(),
            });
        });
    }

    fn confirm_remote_delete(&mut self, target: SftpRemotePaneRef, cx: &mut Context<Self>) {
        let Some(pane) = self.state.remote_pane(target) else {
            return;
        };
        let targets: Vec<RemoteDeleteTarget> = pane
            .selected_paths()
            .iter()
            .filter_map(|path| {
                pane.entries
                    .iter()
                    .find(|entry| &entry.path == path)
                    .map(|entry| RemoteDeleteTarget {
                        path: entry.path.clone(),
                        is_dir: entry.is_dir,
                    })
            })
            .collect();
        if targets.is_empty() {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        }
        let side_connection = match target {
            SftpRemotePaneRef::Main => None,
            SftpRemotePaneRef::Side(connection_id) => Some(connection_id),
        };
        let tab_id = self.tab_id;
        let count = targets.len();
        let app = self.app.clone();
        with_shell_parts(&app, cx, |shell, _| {
            shell.state.sftp_modals.confirm = Some(SftpConfirmModalState {
                action: SftpPendingDelete::Remote {
                    tab_id,
                    connection_id: side_connection,
                    targets,
                },
                title: rust_i18n::t!("sftp.file_ops.delete_remote_title").to_string(),
                description: rust_i18n::t!("sftp.file_ops.delete_remote_body", count = count)
                    .to_string(),
            });
        });
    }
}
