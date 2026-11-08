//! SFTP 文件系统增删改查、模态对话框交互及软链接操作。

mod commit;
mod validate;

use crate::app::AppShell;
use crate::sftp::SftpPendingDelete;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteDeleteTarget, SftpOperation, TabId};

impl AppShell {
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
            panel.state.set_status(
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
}
