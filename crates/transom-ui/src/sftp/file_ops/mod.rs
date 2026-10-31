//! SFTP 文件系统增删改查、模态对话框交互及软链接操作。

mod commit;
mod remote_inputs;
mod validate;

use crate::app::AppShell;
use crate::sftp::{SftpConfirmModalState, SftpPendingDelete, SftpRemotePaneRef};
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteDeleteTarget, SftpOperation, TabId};

impl AppShell {
    /// 批量删除目标远端列选中项：先弹确认，确认后再按递归语义删除。
    pub(crate) fn sftp_delete_selection_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some((targets, side_connection)) =
            self.remote_pane_scope(tab_id, target, cx).map(|scope| {
                let targets = scope
                    .pane
                    .selected_paths()
                    .iter()
                    .filter_map(|path| {
                        scope
                            .pane
                            .entries
                            .iter()
                            .find(|entry| &entry.path == path)
                            .map(|entry| RemoteDeleteTarget {
                                path: entry.path.clone(),
                                is_dir: entry.is_dir,
                            })
                    })
                    .collect::<Vec<_>>();
                (targets, scope.side_connection)
            })
        else {
            return;
        };
        if targets.is_empty() {
            self.set_sftp_pane_error(
                tab_id,
                target,
                rust_i18n::t!("sftp.select_file").to_string(),
                cx,
            );
            return;
        }
        let count = targets.len();
        self.state.sftp_modals.confirm = Some(SftpConfirmModalState {
            action: SftpPendingDelete::Remote {
                tab_id,
                connection_id: side_connection,
                targets,
            },
            title: rust_i18n::t!("sftp.file_ops.delete_remote_title").to_string(),
            description: rust_i18n::t!("sftp.file_ops.delete_remote_body", count = count)
                .to_string(),
        });
        cx.notify();
    }

    /// 执行已确认的远端删除。
    pub(crate) fn perform_sftp_delete(
        &mut self,
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        targets: Vec<RemoteDeleteTarget>,
        cx: &mut Context<Self>,
    ) {
        let count = targets.len();
        let Some(connection_id) = connection_id.or_else(|| {
            self.state
                .workspace
                .tab(tab_id)
                .and_then(|tab| tab.connection_id)
        }) else {
            return;
        };
        self.send_sftp_op_for_connection(
            tab_id,
            connection_id,
            SftpOperation::DeleteMany { targets },
            cx,
        );
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
            panel.set_status(
                rust_i18n::t!("sftp.file_ops.deleting_remote", count = count).to_string(),
            );
        }
        cx.notify();
    }

    /// 执行确认弹窗中挂起的删除动作。
    pub(crate) fn sftp_confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.sftp_modals.confirm.take() else {
            return;
        };
        match modal.action {
            SftpPendingDelete::Local { tab_id, paths } => {
                self.perform_sftp_local_delete(tab_id, paths, cx);
            }
            SftpPendingDelete::Remote {
                tab_id,
                connection_id,
                targets,
            } => {
                self.perform_sftp_delete(tab_id, connection_id, targets, cx);
            }
        }
    }

    /// Delete 快捷键：删除当前活动栏选中的项目（与右键"删除"行为一致，均先弹确认）。
    ///
    /// 设计原理（Why）：双栏共用同一键盘上下文，因此以"最近交互栏"（`active_pane`）
    /// 确定删除目标；与右键菜单删除共用同一操作路径，活动栏由统一选中方法维护，
    /// 不会出现删错栏位的失配。删除不可恢复，确认步骤由
    /// `sftp_delete_selection` 与 `sftp_local_delete_selection` 统一弹出。
    pub(crate) fn sftp_open_local_mkdir_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.open_local_mkdir_modal(window, cx));
        }
    }

    pub(crate) fn sftp_open_mkdir_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.open_mkdir_modal_for(target, window, cx));
        }
    }

    pub(crate) fn sftp_open_editor_new_file_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.open_editor_new_file_modal_for(target, window, cx)
            });
        }
    }

    pub(crate) fn sftp_open_local_new_file_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.open_local_new_file_modal(window, cx));
        }
    }

    pub(crate) fn sftp_open_local_rename_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.open_local_rename_modal(window, cx));
        }
    }

    pub(crate) fn sftp_open_rename_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.open_rename_modal_for(target, window, cx)
            });
        }
    }

    pub(crate) fn sftp_open_local_symlink_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.open_local_symlink_modal(window, cx));
        }
    }
}
