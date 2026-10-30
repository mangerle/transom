//! SFTP 文件系统增删改查、模态对话框交互及软链接操作。

mod commit;
mod remote_inputs;
mod validate;

use crate::app::AppShell;
use crate::keymap;
use crate::sftp::{
    SftpActivePane, SftpConfirmModalState, SftpInputModalParams, SftpInputTarget,
    SftpPendingDelete, SftpRemotePaneRef,
};
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
            self.remote_pane_scope(tab_id, target).map(|scope| {
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
        self.sftp_modals.confirm = Some(SftpConfirmModalState {
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
        let Some(connection_id) =
            connection_id.or_else(|| self.workspace.tab(tab_id).and_then(|tab| tab.connection_id))
        else {
            return;
        };
        self.send_sftp_op_for_connection(
            tab_id,
            connection_id,
            SftpOperation::DeleteMany { targets },
            cx,
        );
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.set_status(
                rust_i18n::t!("sftp.file_ops.deleting_remote", count = count).to_string(),
            );
            cx.notify();
        }
    }

    /// 执行确认弹窗中挂起的删除动作。
    pub(crate) fn sftp_confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_modals.confirm.take() else {
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
    pub(crate) fn sftp_delete_selected(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(active_pane) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| panel.active_pane)
        else {
            return;
        };
        match active_pane {
            SftpActivePane::Remote => {
                self.sftp_delete_selection_for(tab_id, SftpRemotePaneRef::Main, cx)
            }
            SftpActivePane::Local => self.sftp_local_delete_selection(tab_id, cx),
            SftpActivePane::SideRemote(connection_id) => {
                self.sftp_delete_selection_for(tab_id, SftpRemotePaneRef::Side(connection_id), cx)
            }
        }
    }

    pub(crate) fn on_delete_sftp_selection(
        &mut self,
        _: &keymap::DeleteSftpSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.workspace.active_tab {
            self.sftp_delete_selected(tab_id, cx);
        }
    }

    pub(crate) fn sftp_open_local_mkdir_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: None,
                title: rust_i18n::t!("sftp.file_ops.mkdir_local_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.mkdir_local_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.folder_name_placeholder").to_string(),
                value: String::new(),
                target: SftpInputTarget::LocalMkdir,
            },
            window,
            cx,
        );
    }

    /// 打开目标远端列的新建目录弹窗。
    pub(crate) fn sftp_open_mkdir_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(side_connection) = self
            .remote_pane_scope(tab_id, target)
            .map(|scope| scope.side_connection)
        else {
            return;
        };
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: side_connection,
                title: rust_i18n::t!("sftp.file_ops.mkdir_remote_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.mkdir_remote_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.folder_name_placeholder").to_string(),
                value: String::new(),
                target: SftpInputTarget::RemoteMkdir,
            },
            window,
            cx,
        );
    }

    /// 打开"新建远端文件并编辑"输入模态：提交后由编辑器打开空面板，
    /// 首次保存才在远端创建文件。
    pub(crate) fn sftp_open_editor_new_file_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(side_connection) = self
            .remote_pane_scope(tab_id, target)
            .map(|scope| scope.side_connection)
        else {
            return;
        };
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: side_connection,
                title: rust_i18n::t!("sftp.file_ops.new_remote_file_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.new_remote_file_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.new_file_placeholder").to_string(),
                value: String::new(),
                target: SftpInputTarget::RemoteEditorNewFile,
            },
            window,
            cx,
        );
    }

    /// 打开"新建本地文件"输入模态：提交后在本地当前目录下创建该空文件。
    pub(crate) fn sftp_open_local_new_file_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: None,
                title: rust_i18n::t!("sftp.file_ops.new_local_file_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.new_local_file_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.new_file_placeholder").to_string(),
                value: String::new(),
                target: SftpInputTarget::LocalNewFile,
            },
            window,
            cx,
        );
    }

    pub(crate) fn sftp_open_local_rename_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| panel.local.selected.clone())
        else {
            return;
        };
        let initial = selected
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: None,
                title: rust_i18n::t!("sftp.file_ops.rename_local_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.rename_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.rename_placeholder").to_string(),
                value: initial,
                target: SftpInputTarget::LocalRename { old_path: selected },
            },
            window,
            cx,
        );
    }

    /// 打开目标远端列的重命名弹窗，初始值取当前主项的文件名。
    pub(crate) fn sftp_open_rename_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((selected, side_connection)) = self
            .remote_pane_scope(tab_id, target)
            .and_then(|scope| Some((scope.pane.selected.clone()?, scope.side_connection)))
        else {
            self.set_sftp_pane_error(
                tab_id,
                target,
                rust_i18n::t!("sftp.select_file").to_string(),
                cx,
            );
            return;
        };
        let initial = selected
            .trim_end_matches('/')
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| selected.clone());
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: side_connection,
                title: rust_i18n::t!("sftp.file_ops.rename_remote_title").to_string(),
                prompt: rust_i18n::t!("sftp.file_ops.rename_prompt").to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.rename_placeholder").to_string(),
                value: initial,
                target: SftpInputTarget::RemoteRename { old_path: selected },
            },
            window,
            cx,
        );
    }

    /// 打开本地「创建软链接」输入弹窗：基于当前选中项目。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：创建软链接需要用户为新链接命名，且必须明确被指向的目标路径，
    ///   因此复用通用输入弹窗承载「名称输入 + 目标信息提示」；
    /// - 核心优势：默认名称在所选项目名后追加 `.link` 后缀，避免与既有项目重名
    ///   导致服务端直接拒绝，用户也可按需改写；
    /// - 代价与局限：链接固定创建于当前目录，暂不支持跨目录指定链接存放位置。
    pub(crate) fn sftp_open_local_symlink_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| panel.local.selected.clone())
        else {
            return;
        };
        let initial_name = selected
            .file_name()
            .map(|name| format!("{}.link", name.to_string_lossy()))
            .unwrap_or_else(|| "link".to_string());
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: None,
                title: rust_i18n::t!("sftp.file_ops.symlink_local_title").to_string(),
                prompt: rust_i18n::t!(
                    "sftp.file_ops.symlink_local_prompt",
                    path = selected.display().to_string()
                )
                .to_string(),
                placeholder: rust_i18n::t!("sftp.file_ops.symlink_name_placeholder").to_string(),
                value: initial_name,
                target: SftpInputTarget::LocalSymlink {
                    target_path: selected,
                },
            },
            window,
            cx,
        );
    }
}
