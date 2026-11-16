//! 弹窗状态判定、取消与提交。
//!
//! 这些方法需要读取或修改 AppShell 的私有状态，因此作为 dialog_host 的子模块
//! 存在；弹窗种类与元数据由 `registry` 统一登记，新增弹窗时先改变体再登记。

use super::*;

impl ModalKind {
    /// 该弹窗此刻是否应当存在；容器工作区弹窗由 [`ContainerWorkspaceModal`] 统一接管，
    /// 其余种类由注册表按种类提供。
    pub(super) fn is_open(self, shell: &AppShell, cx: &App) -> bool {
        if let Some(modal) = self.as_container_modal() {
            return crate::container::find_container_modal_tab(shell, cx, |ws| modal.is_open(ws))
                .is_some();
        }
        let state = super::registry::DialogCheckState::from_state(&shell.state);
        (super::registry::entry(self).is_open)(&state, cx)
    }

    /// 是否存在未提交内容：与影子栈记录的打开时基线逐项比对。
    ///
    /// 为什么用基线快照而不是"用户改过就算脏"的标记位：文本框的值由组件自己
    /// 维护，逐处加脏标记要把每个输入框都接一遍回调，漏一处就出现"改了不拦"；
    /// 快照比对只看结果，新增字段不会漏。代价是每次取消都要读一遍全部文本项，
    /// 量级是二十来个字符串，可忽略。
    pub(super) fn is_dirty(self, shell: &AppShell, cx: &App) -> bool {
        let current = self.field_values(shell, cx);
        !current.is_empty() && current != shell.state.open_modals.baseline(self)
    }

    /// 该种类需要跟踪的文本项；返回空表示不参与脏判定（确认与授权类）。
    pub(super) fn field_values(self, shell: &AppShell, cx: &App) -> Vec<String> {
        if let Some(modal) = self.as_container_modal() {
            return crate::container::find_container_modal_tab(shell, cx, |ws| modal.is_open(ws))
                .map(|(_, ws)| modal.text_values(&ws.read(cx).state, cx))
                .unwrap_or_default();
        }
        let state = super::registry::DialogCheckState::from_state(&shell.state);
        (super::registry::entry(self).field_values)(&state, cx)
    }

    /// 取消：把用户意图回给等待方并清空自身状态，返回是否允许容器关闭该层。
    ///
    /// 授权类必须走各自的 cancel 方法把结果发回 responder：只清状态会让
    /// 等待裁决的在途连接任务永久挂住。
    /// 返回 `true` 表示取消已被执行、这一层允许关闭；脏表单返回 `false` 并写提示。
    pub(super) fn cancel(
        self,
        shell: &mut AppShell,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool {
        if self.is_dirty(shell, cx) {
            // 拦截必须可见：文案说明为什么按了没反应，行为与提示一致。
            shell.state.feedback.dialog_hint =
                Some(rust_i18n::t!("dialog.dirty_block_hint").to_string());
            cx.notify();
            return false;
        }
        shell.state.feedback.dialog_hint = None;
        if let Some(handled) = self.container_cancel(shell, cx) {
            return handled;
        }
        if let Some(handled) = self.git_cancel(shell, cx) {
            return handled;
        }
        match self {
            ModalKind::SftpConfirm
            | ModalKind::HostKeyConfirm
            | ModalKind::ClipboardPrompt
            | ModalKind::ForwardConfirm
            | ModalKind::AuthPrompt
            | ModalKind::SftpInput
            | ModalKind::SftpChmod
            | ModalKind::SftpProperties
            | ModalKind::SftpFilter
            | ModalKind::SftpSearch
            | ModalKind::SftpSync => self.cancel_prompt_or_sftp(shell, window, cx),
            _ => self.cancel_tools_and_session(shell, window, cx),
        }
    }

    /// 取消连接鉴权提示与 SFTP 弹窗。
    fn cancel_prompt_or_sftp(
        self,
        shell: &mut AppShell,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool {
        match self {
            ModalKind::SftpConfirm => {
                shell.state.modals.sftp.close_confirm();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            ModalKind::HostKeyConfirm => {
                shell
                    .state
                    .modals
                    .connection
                    .prompts
                    .update(cx, |view, cx| view.reject_host_key(cx));
                true
            }
            ModalKind::ClipboardPrompt => {
                shell
                    .state
                    .modals
                    .connection
                    .prompts
                    .update(cx, |view, cx| view.deny_clipboard(cx));
                true
            }
            ModalKind::ForwardConfirm => {
                crate::forward::cancel_forward_confirm(shell, cx);
                true
            }
            ModalKind::AuthPrompt => {
                shell
                    .state
                    .modals
                    .connection
                    .prompts
                    .update(cx, |view, cx| view.cancel_auth(cx));
                true
            }
            ModalKind::SftpInput => {
                shell.state.modals.sftp.close_input();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            ModalKind::SftpChmod => {
                shell.state.modals.sftp.close_chmod();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            ModalKind::SftpProperties => {
                shell.state.modals.sftp.close_properties();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            ModalKind::SftpFilter => {
                shell.state.modals.sftp.close_filter();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            ModalKind::SftpSearch => {
                shell.state.modals.sftp.close_search();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            ModalKind::SftpSync => {
                shell.state.modals.sftp.close_sync();
                cx.notify();
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
                true
            }
            _ => true,
        }
    }

    /// 取消编辑器、容器表单、会话与工具弹窗。
    fn cancel_tools_and_session(
        self,
        shell: &mut AppShell,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool {
        match self {
            ModalKind::EditorGotoLine => {
                crate::editor::cancel_editor_goto_line(shell, window, cx);
                true
            }
            ModalKind::ContainerForm => {
                shell
                    .state
                    .modals
                    .tools
                    .container_form
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::ContainerFolderForm => {
                shell
                    .state
                    .containers
                    .sidebar
                    .update(cx, |sidebar, cx| sidebar.dismiss_folder_form(cx));
                true
            }
            ModalKind::ContainerRunForm => {
                shell
                    .state
                    .modals
                    .tools
                    .run_form
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::ContainerCreate => {
                shell
                    .state
                    .modals
                    .tools
                    .create_dialog
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::SessionForm => {
                shell
                    .state
                    .modals
                    .session
                    .form
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::FolderForm => {
                shell.state.modals.session.folder_form = None;
                true
            }
            ModalKind::BatchEdit => {
                shell.state.modals.session.batch_edit = None;
                true
            }
            ModalKind::SnippetForm => {
                shell.state.modals.tools.snippet_panel.form = None;
                true
            }
            ModalKind::KnownHosts => {
                shell
                    .state
                    .modals
                    .connection
                    .known_hosts
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::KeyWizard => {
                shell
                    .state
                    .modals
                    .tools
                    .key_wizard
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::BackupPrompt => {
                shell
                    .state
                    .modals
                    .tools
                    .backup_prompt
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::AuditViewer => {
                shell
                    .state
                    .modals
                    .tools
                    .audit_viewer
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::Replay => {
                shell
                    .state
                    .modals
                    .tools
                    .replay
                    .update(cx, |view, cx| view.close(cx));
                true
            }
            ModalKind::Settings => {
                shell.cancel_settings(window, cx);
                true
            }
            ModalKind::ShortcutCapture => {
                shell.state.shortcut_recording.capture = None;
                true
            }
            ModalKind::DiffViewer => {
                crate::diff_viewer::close_diff_viewer(shell, cx);
                true
            }
            _ => true,
        }
    }
}

mod commit;
