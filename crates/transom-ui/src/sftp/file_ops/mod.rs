//! SFTP 文件系统增删改查、模态对话框交互及软链接操作。

mod commit;
pub(crate) use commit::commit_sftp_input_modal;
mod validate;

use crate::app::AppShell;
use crate::sftp::SftpPendingDelete;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteDeleteTarget, SftpOperation, TabId};

/// 执行已确认的远端删除。
pub(crate) fn perform_sftp_delete(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: Option<ConnectionId>,
    targets: Vec<RemoteDeleteTarget>,
    cx: &mut Context<AppShell>,
) {
    let count = targets.len();
    let Some(connection_id) = connection_id.or_else(|| {
        shell
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id)
    }) else {
        return;
    };
    super::remote_actions::send_sftp_op_for_connection(
        shell,
        tab_id,
        connection_id,
        SftpOperation::DeleteMany { targets },
        cx,
    );
    if let Some(mut panel) = crate::sftp::sftp_view_mut(shell, tab_id, cx) {
        panel.state.set_status(
            rust_i18n::t!("sftp.file_ops.deleting_remote", count = count).to_string(),
        );
    }
    cx.notify();
}

/// 执行确认弹窗中挂起的删除动作。
pub(crate) fn sftp_confirm_delete(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let Some(modal) = shell.state.sftp_modals.confirm.take() else {
        return;
    };
    match modal.action {
        SftpPendingDelete::Local { tab_id, paths } => {
            crate::sftp::perform_sftp_local_delete(shell, tab_id, paths, cx);
        }
        SftpPendingDelete::Remote {
            tab_id,
            connection_id,
            targets,
        } => {
            perform_sftp_delete(shell, tab_id, connection_id, targets, cx);
        }
    }
}
