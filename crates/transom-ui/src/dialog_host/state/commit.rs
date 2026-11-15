//! 弹窗提交与焦点目标解析。

use super::super::body::ModalFocus;
use super::*;

impl ModalKind {
    /// 提交：等价于点主按钮。
    ///
    /// 各领域方法自己负责把状态置回 `None`（提交成功）或保留弹窗并写错误：
    /// 关闭与否一律交给对账器按期望栈收敛，不让容器先弹栈，否则校验失败时
    /// 状态仍在、下一帧又按状态把弹窗重开一次。
    ///
    /// 提交：执行主按钮动作，然后按"该弹窗状态是否已消失"决定是否允许关闭。
    ///
    /// 为什么用状态反查而不是让每个领域返回布尔：校验失败、保存失败、需要继续
    /// 填的表单都会把状态留在原处，动作函数本身并不统一返回成败；而"状态还在"
    /// 恰恰就是弹窗该继续存在的唯一事实。
    pub(in super::super) fn commit(
        self,
        shell: &mut AppShell,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool {
        self.run_commit(shell, window, cx);
        !self.is_open(shell, cx)
    }

    fn run_commit(self, shell: &mut AppShell, window: &mut Window, cx: &mut Context<AppShell>) {
        if self.container_commit(shell, window, cx).is_some() {
            return;
        }
        if self.git_commit(shell, window, cx).is_some() {
            return;
        }
        match self {
            ModalKind::SftpConfirm => {
                if let Some(modal) = shell.state.sftp_modals.confirm.clone() {
                    modal.update(cx, |m, cx| m.submit(cx));
                }
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            ModalKind::HostKeyConfirm => {
                shell
                    .state
                    .modals
                    .connection
                    .prompts
                    .update(cx, |view, cx| view.accept_host_key(cx));
            }
            ModalKind::ClipboardPrompt => {
                shell
                    .state
                    .modals
                    .connection
                    .prompts
                    .update(cx, |view, cx| view.allow_clipboard(cx));
            }
            ModalKind::ForwardConfirm => crate::forward::confirm_forward_start(shell, cx),
            ModalKind::AuthPrompt => {
                shell
                    .state
                    .modals
                    .connection
                    .prompts
                    .update(cx, |view, cx| view.submit_auth(cx));
            }
            ModalKind::SftpInput => {
                if let Some(modal) = shell.state.sftp_modals.input.clone() {
                    modal.update(cx, |m, cx| m.submit(window, cx));
                }
            }
            ModalKind::SftpChmod => {
                if let Some(modal) = shell.state.sftp_modals.chmod.clone() {
                    modal.update(cx, |m, cx| m.submit(window, cx));
                }
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            // 属性与搜索弹窗只有"关闭"这一个主按钮，Enter 与它同义。
            ModalKind::SftpProperties => {
                crate::sftp::close_sftp_properties_modal(shell, cx);
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            ModalKind::SftpSearch => {
                crate::sftp::close_sftp_search_modal(shell, cx);
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            ModalKind::SftpFilter => {
                if let Some(modal) = shell.state.sftp_modals.filter.clone() {
                    modal.update(cx, |m, cx| m.submit(window, cx));
                }
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            ModalKind::SftpSync => {
                if let Some(modal) = shell.state.sftp_modals.sync.clone() {
                    modal.update(cx, |m, cx| m.submit(cx));
                }
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            ModalKind::EditorGotoLine => crate::editor::commit_editor_goto_line(shell, window, cx),
            ModalKind::ContainerForm => {
                shell
                    .state
                    .modals
                    .tools
                    .container_form
                    .update(cx, |view, cx| view.save(cx));
            }
            ModalKind::ContainerFolderForm => {
                shell
                    .state
                    .containers
                    .sidebar
                    .update(cx, |sidebar, cx| sidebar.submit_folder_form(cx));
            }
            ModalKind::ContainerRunForm => {
                shell
                    .state
                    .modals
                    .tools
                    .run_form
                    .update(cx, |view, cx| view.submit(cx));
            }
            ModalKind::ContainerCreate => {
                shell
                    .state
                    .modals
                    .tools
                    .create_dialog
                    .update(cx, |view, cx| view.submit(cx));
            }
            ModalKind::SessionForm => {
                shell
                    .state
                    .modals
                    .session
                    .form
                    .update(cx, |view, cx| view.save(cx));
            }
            ModalKind::FolderForm => crate::sidebar::save_folder_form(shell, cx),
            ModalKind::BatchEdit => crate::batch::save_batch_edit(shell, cx),
            ModalKind::SnippetForm => crate::snippets::save_snippet_form(shell, cx),
            ModalKind::BackupPrompt => {
                shell
                    .state
                    .modals
                    .tools
                    .backup_prompt
                    .update(cx, |view, cx| view.submit(window, cx));
            }
            // 查看与管理类弹窗只有一个"关闭"主按钮，Enter 与它同义。
            ModalKind::KnownHosts => shell
                .state
                .modals
                .connection
                .known_hosts
                .update(cx, |view, cx| view.close(cx)),
            ModalKind::KeyWizard => shell
                .state
                .modals
                .tools
                .key_wizard
                .update(cx, |view, cx| view.close(cx)),
            ModalKind::AuditViewer => shell
                .state
                .modals
                .tools
                .audit_viewer
                .update(cx, |view, cx| view.close(cx)),
            // Enter 与原实现一致只关闭弹窗；打开播放器由页脚主按钮显式触发。
            ModalKind::Replay => shell
                .state
                .modals
                .tools
                .replay
                .update(cx, |view, cx| view.close(cx)),
            ModalKind::Settings => shell.save_settings(window, cx),
            ModalKind::ShortcutCapture => shell.state.shortcut_recording.capture = None,
            ModalKind::DiffViewer => crate::diff_viewer::close_diff_viewer(shell, cx),
            _ => {}
        }
    }

    /// 首帧焦点：文本类弹窗把焦点交给自己的输入控件，选择类交给外壳。
    ///
    /// 一律走 `as_ref()` 安全取值：关闭动作可能在对账把弹窗弹出栈之前就把状态
    /// 清空，这一帧仍可能渲染一次；用 `expect` 会把这条正常路径变成 panic。
    pub(in super::super) fn focus_target(self, shell: &AppShell, cx: &App) -> ModalFocus {
        if let Some(target) = self.container_focus_target(shell, cx) {
            return target;
        }
        if let Some(target) = self.git_focus_target(shell) {
            return target;
        }
        match self {
            ModalKind::SftpInput => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .input
                    .as_ref()
                    .map(|modal| modal.read(cx).input.clone())
            }),
            ModalKind::SftpChmod => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .chmod
                    .as_ref()
                    .map(|modal| modal.read(cx).octal_input.clone())
            }),
            ModalKind::SftpSearch => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .search
                    .as_ref()
                    .map(|modal| modal.read(cx).state.input.clone())
            }),
            ModalKind::EditorGotoLine => input_focus(shell, |state| {
                state
                    .modals
                    .tools
                    .editor_goto_line
                    .as_ref()
                    .map(|modal| modal.input.clone())
            }),
            ModalKind::ContainerForm => shell
                .state
                .modals
                .tools
                .container_form
                .read(cx)
                .state()
                .map_or(ModalFocus::None, |form| {
                    ModalFocus::Input(form.name.clone())
                }),
            ModalKind::ContainerFolderForm => shell
                .state
                .containers
                .sidebar
                .read(cx)
                .state
                .engine_folder_form
                .as_ref()
                .map_or(ModalFocus::None, |form| {
                    ModalFocus::Input(form.input.clone())
                }),
            ModalKind::ContainerRunForm => shell
                .state
                .modals
                .tools
                .run_form
                .read(cx)
                .state()
                .map_or(ModalFocus::None, |form| {
                    ModalFocus::Input(form.image.clone())
                }),
            ModalKind::ContainerCreate => {
                let input = shell
                    .state
                    .modals
                    .tools
                    .create_dialog
                    .read(cx)
                    .state()
                    .and_then(|dialog| {
                        let workspace = shell.state.container_workspace(dialog.tab_id)?;
                        Some(workspace.read(cx).state.create_input.clone())
                    });
                input.map_or(ModalFocus::None, ModalFocus::Input)
            }
            ModalKind::SessionForm => shell
                .state
                .modals
                .session
                .form
                .read(cx)
                .state()
                .map_or(ModalFocus::None, |form| {
                    ModalFocus::Input(form.name.clone())
                }),
            ModalKind::FolderForm => input_focus(shell, |state| {
                state
                    .modals
                    .session
                    .folder_form
                    .as_ref()
                    .map(|form| form.name.clone())
            }),
            ModalKind::SnippetForm => input_focus(shell, |state| {
                state
                    .modals
                    .tools
                    .snippet_panel
                    .form
                    .as_ref()
                    .map(|form| form.name_input())
            }),
            ModalKind::SftpFilter => shell
                .state
                .sftp_modals
                .filter
                .as_ref()
                .map_or(ModalFocus::None, |modal| {
                    ModalFocus::Textarea(modal.read(cx).textarea.clone())
                }),
            // 改键录入面板自己持有焦点句柄：录键期间按键要落在这块面板上。
            ModalKind::ShortcutCapture => shell
                .state
                .shortcut_recording
                .capture
                .as_ref()
                .map_or(ModalFocus::None, |capture| {
                    ModalFocus::Handle(capture.focus.clone())
                }),
            _ => ModalFocus::None,
        }
    }
}

/// 从外壳状态取输入控件作为首帧焦点；缺失时保持无焦点。
fn input_focus(
    shell: &AppShell,
    pick: impl Fn(&crate::app::AppState) -> Option<Entity<InputState>>,
) -> ModalFocus {
    pick(&shell.state).map_or(ModalFocus::None, ModalFocus::Input)
}
