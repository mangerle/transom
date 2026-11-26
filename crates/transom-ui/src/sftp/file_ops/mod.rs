//! SFTP 文件系统增删改查、模态对话框交互及软链接操作。

mod commit;
pub(in crate::app::sftp) use commit::{SftpInputCommit, commit_sftp_input_value};
pub(in crate::app::sftp) mod validate;

use crate::app::{AppShell, AppState};
use crate::sftp::SftpPendingDelete;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteDeleteTarget, SftpOperation, TabId};

/// 执行已确认的远端删除。
pub(crate) fn perform_sftp_delete(
    app_state: &mut AppState,
    tab_id: TabId,
    connection_id: Option<ConnectionId>,
    targets: Vec<RemoteDeleteTarget>,
    cx: &mut Context<AppShell>,
) {
    let count = targets.len();
    let Some(connection_id) = connection_id.or_else(|| {
        app_state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id())
    }) else {
        return;
    };
    crate::sftp::send_sftp_op_for_connection(
        app_state,
        tab_id,
        connection_id,
        SftpOperation::DeleteMany { targets },
        cx,
    );
    if let Some(mut panel) = crate::sftp::sftp_view_mut(&app_state.workspace, tab_id, cx) {
        panel
            .state
            .set_status(rust_i18n::t!("sftp.file_ops.deleting_remote", count = count).to_string());
    }
    cx.notify();
}

/// 执行确认弹窗中挂起的删除动作。
pub(crate) fn execute_sftp_delete(
    app_state: &mut AppState,
    action: SftpPendingDelete,
    cx: &mut Context<AppShell>,
) {
    match action {
        SftpPendingDelete::Local { tab_id, paths } => {
            if let Some(view) = app_state.sftp_view(tab_id) {
                view.update(cx, |view, cx| view.perform_local_delete(paths, cx));
            }
        }
        SftpPendingDelete::Remote {
            tab_id,
            connection_id,
            targets,
        } => {
            perform_sftp_delete(app_state, tab_id, connection_id, targets, cx);
        }
    }
}
