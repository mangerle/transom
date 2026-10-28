//! SFTP 文件系统增删改查、模态对话框交互及软链接操作。

mod commit;
mod validate;

use crate::app::AppShell;
use crate::keymap;
use crate::palette::sftp_text_input;
use crate::sftp::{
    SftpActivePane, SftpConfirmModalState, SftpInputModalState, SftpInputTarget, SftpPendingDelete,
};
use gpui_kit::*;
use transom_core::{RemoteDeleteTarget, SftpOperation, TabId};

impl AppShell {
    /// 批量删除远端选中项：先弹确认，确认后再按递归语义删除。
    pub(crate) fn sftp_delete_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        let paths = panel.selected_remote_paths();
        if paths.is_empty() {
            panel.set_error(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let targets = paths
            .iter()
            .filter_map(|path| {
                panel
                    .remote
                    .entries
                    .iter()
                    .find(|entry| &entry.path == path)
                    .map(|entry| RemoteDeleteTarget {
                        path: entry.path.clone(),
                        is_dir: entry.is_dir,
                    })
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            panel.set_error(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let count = targets.len();
        self.sftp_modals.confirm = Some(SftpConfirmModalState {
            action: SftpPendingDelete::Remote { tab_id, targets },
            title: "删除远端项目".to_string(),
            description: format!(
                "将永久删除 {count} 个远端项目（目录按递归删除），此操作不可撤销。"
            ),
        });
        cx.notify();
    }

    /// 执行已确认的远端删除。
    pub(crate) fn perform_sftp_delete(
        &mut self,
        tab_id: TabId,
        targets: Vec<RemoteDeleteTarget>,
        cx: &mut Context<Self>,
    ) {
        let count = targets.len();
        self.send_sftp_op(tab_id, SftpOperation::DeleteMany { targets }, cx);
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.set_status(format!("正在删除 {count} 个远端项目"));
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
            SftpPendingDelete::Remote { tab_id, targets } => {
                self.perform_sftp_delete(tab_id, targets, cx);
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
            SftpActivePane::Remote => self.sftp_delete_selection(tab_id, cx),
            SftpActivePane::Local => self.sftp_local_delete_selection(tab_id, cx),
            SftpActivePane::SideRemote(_) => {}
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

    pub(crate) fn sftp_open_mkdir_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, prompt, target) = if is_remote {
            (
                "新建远端文件夹".to_string(),
                "请输入新建远端文件夹的名称:".to_string(),
                SftpInputTarget::RemoteMkdir,
            )
        } else {
            (
                "新建本地文件夹".to_string(),
                "请输入新建本地文件夹的名称:".to_string(),
                SftpInputTarget::LocalMkdir,
            )
        };
        let input = sftp_text_input(window, cx, "", "文件夹名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    /// 打开"新建远端文件并编辑"输入模态：提交后由编辑器打开空面板，
    /// 首次保存才在远端创建文件。
    pub(crate) fn sftp_open_editor_new_file_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = sftp_text_input(window, cx, "", "新文件名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title: "新建远端文件".to_string(),
            prompt: "输入新文件名称（在当前远端目录下创建）:".to_string(),
            input,
            target: SftpInputTarget::RemoteEditorNewFile,
            error: None,
        });
        cx.notify();
    }

    /// 打开"新建本地文件"输入模态：提交后在本地当前目录下创建该空文件。
    pub(crate) fn sftp_open_local_new_file_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = sftp_text_input(window, cx, "", "新文件名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title: "新建本地文件".to_string(),
            prompt: "输入新文件名称（在当前本地目录下创建）:".to_string(),
            input,
            target: SftpInputTarget::LocalNewFile,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn sftp_open_rename_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp() else {
            return;
        };
        let (title, prompt, initial_value, target) = if is_remote {
            let Some(selected) = panel.remote.selected.clone() else {
                return;
            };
            let initial = selected
                .trim_end_matches('/')
                .rsplit_once('/')
                .map(|(_, name)| name.to_string())
                .unwrap_or_else(|| selected.clone());
            (
                "重命名远端文件".to_string(),
                "请输入新的名称:".to_string(),
                initial,
                SftpInputTarget::RemoteRename { old_path: selected },
            )
        } else {
            let Some(selected) = panel.local.selected.clone() else {
                return;
            };
            let initial = selected
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (
                "重命名本地文件".to_string(),
                "请输入新的名称:".to_string(),
                initial,
                SftpInputTarget::LocalRename { old_path: selected },
            )
        };
        let input = sftp_text_input(window, cx, &initial_value, "新名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    /// 打开「创建软链接」输入弹窗：远端基于当前选中条目，本地基于当前选中项目。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：创建软链接需要用户为新链接命名，且必须明确被指向的目标路径，
    ///   因此复用通用输入弹窗承载「名称输入 + 目标信息提示」；
    /// - 核心优势：默认名称在所选项目名后追加 `.link` 后缀，避免与既有项目重名
    ///   导致服务端直接拒绝，用户也可按需改写；
    /// - 代价与局限：链接固定创建于当前目录，暂不支持跨目录指定链接存放位置。
    pub(crate) fn sftp_open_symlink_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp() else {
            return;
        };
        let (title, prompt, initial_value, target) = if is_remote {
            let Some(selected) = panel.remote.selected.clone() else {
                return;
            };
            let Some(entry) = panel
                .remote
                .entries
                .iter()
                .find(|entry| entry.path == selected)
            else {
                return;
            };
            (
                "创建远端软链接".to_string(),
                format!("新链接将创建于当前远端目录，指向: {}", entry.path),
                format!("{}.link", entry.name),
                SftpInputTarget::RemoteSymlink {
                    target: entry.path.clone(),
                },
            )
        } else {
            let Some(selected) = panel.local.selected.clone() else {
                return;
            };
            let initial_name = selected
                .file_name()
                .map(|name| format!("{}.link", name.to_string_lossy()))
                .unwrap_or_else(|| "link".to_string());
            (
                "创建本地软链接".to_string(),
                format!("新链接将创建于当前本地目录，指向: {}", selected.display()),
                initial_name,
                SftpInputTarget::LocalSymlink {
                    target_path: selected,
                },
            )
        };
        let input = sftp_text_input(window, cx, &initial_value, "软链接名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }
}
