//! 实体侧的文件操作入口：收集目标信息并打开输入/归档弹窗。
//!
//! 输入弹窗的装配仍在外壳（需要窗口与 InputState），因此实体只负责从自身
//! 状态算出参数，再经 `with_shell_parts` 回交外壳；外壳方法不读写实体。

use super::*;
use crate::sftp::{SftpInputModalParams, SftpInputTarget, SftpRemotePaneRef};

impl SftpPanelView {
    fn open_input_modal(
        &self,
        params: SftpInputModalParams,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(crate::sftp::SftpPanelEvent::OpenInputModal(params));
        cx.notify();
    }

    fn side_connection(target: SftpRemotePaneRef) -> Option<transom_core::ConnectionId> {
        match target {
            SftpRemotePaneRef::Main => None,
            SftpRemotePaneRef::Side(connection_id) => Some(connection_id),
        }
    }

    pub(crate) fn open_local_mkdir_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: None,
            title: rust_i18n::t!("sftp.file_ops.mkdir_local_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.mkdir_local_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.folder_name_placeholder").to_string(),
            value: String::new(),
            target: SftpInputTarget::LocalMkdir,
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_mkdir_modal_for(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: Self::side_connection(target),
            title: rust_i18n::t!("sftp.file_ops.mkdir_remote_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.mkdir_remote_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.folder_name_placeholder").to_string(),
            value: String::new(),
            target: SftpInputTarget::RemoteMkdir,
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_editor_new_file_modal_for(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: Self::side_connection(target),
            title: rust_i18n::t!("sftp.file_ops.new_remote_file_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.new_remote_file_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.new_file_placeholder").to_string(),
            value: String::new(),
            target: SftpInputTarget::RemoteEditorNewFile,
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_local_new_file_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: None,
            title: rust_i18n::t!("sftp.file_ops.new_local_file_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.new_local_file_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.new_file_placeholder").to_string(),
            value: String::new(),
            target: SftpInputTarget::LocalNewFile,
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_local_rename_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.state.local.selected.clone() else {
            return;
        };
        let initial = selected
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: None,
            title: rust_i18n::t!("sftp.file_ops.rename_local_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.rename_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.rename_placeholder").to_string(),
            value: initial,
            target: SftpInputTarget::LocalRename { old_path: selected },
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_rename_modal_for(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self
            .state
            .remote_pane(target)
            .and_then(|pane| pane.selected.clone())
        else {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        };
        let initial = selected
            .trim_end_matches('/')
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| selected.clone());
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: Self::side_connection(target),
            title: rust_i18n::t!("sftp.file_ops.rename_remote_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.rename_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.rename_placeholder").to_string(),
            value: initial,
            target: SftpInputTarget::RemoteRename { old_path: selected },
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_local_symlink_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.state.local.selected.clone() else {
            return;
        };
        let initial_name = selected
            .file_name()
            .map(|name| format!("{}.link", name.to_string_lossy()))
            .unwrap_or_else(|| "link".to_string());
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
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
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_archive_modal(
        &mut self,
        target: SftpRemotePaneRef,
        source_paths: Vec<String>,
        default_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent_dir) = self.state.remote_pane(target).map(|pane| pane.path.clone()) else {
            return;
        };
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: Self::side_connection(target),
            title: rust_i18n::t!("sftp.file_ops.archive_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.archive_prompt", count = source_paths.len())
                .to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.archive_placeholder").to_string(),
            value: default_name,
            target: SftpInputTarget::RemoteArchive {
                source_paths,
                parent_dir,
            },
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_extract_modal(
        &mut self,
        target: SftpRemotePaneRef,
        archive_path: String,
        default_dir_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent_dir) = self.state.remote_pane(target).map(|pane| pane.path.clone()) else {
            return;
        };
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: Self::side_connection(target),
            title: rust_i18n::t!("sftp.file_ops.extract_title").to_string(),
            prompt: rust_i18n::t!("sftp.file_ops.extract_prompt").to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.extract_placeholder").to_string(),
            value: default_dir_name,
            target: SftpInputTarget::RemoteExtract {
                archive_path,
                parent_dir,
            },
        };
        self.open_input_modal(params, window, cx);
    }

    pub(crate) fn open_symlink_modal(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.state.remote_pane(target).and_then(|pane| {
            let selected = pane.selected.clone()?;
            pane.entries
                .iter()
                .find(|entry| entry.path == selected)
                .cloned()
        }) else {
            return;
        };
        let params = SftpInputModalParams {
            tab_id: self.tab_id,
            connection_id: Self::side_connection(target),
            title: rust_i18n::t!("sftp.file_ops.symlink_remote_title").to_string(),
            prompt: rust_i18n::t!(
                "sftp.file_ops.symlink_remote_prompt",
                path = entry.path.as_str()
            )
            .to_string(),
            placeholder: rust_i18n::t!("sftp.file_ops.symlink_name_placeholder").to_string(),
            value: format!("{}.link", entry.name),
            target: SftpInputTarget::RemoteSymlink { target: entry.path },
        };
        self.open_input_modal(params, window, cx);
    }
}
