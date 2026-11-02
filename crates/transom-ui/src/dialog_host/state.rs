//! 弹窗状态判定、取消与提交。
//!
//! 这些方法需要读取或修改 AppShell 的私有状态，因此作为 dialog_host 的子模块
//! 存在；枚举与元数据仍留在父模块，新增弹窗时先改 `ModalKind`。

use super::body::ModalFocus;
use super::*;
use crate::app::app_settings;

impl ModalKind {
    /// 该弹窗此刻是否应当存在。只读状态、不产生副作用：对账每帧都会问一次。
    pub(super) fn is_open(self, shell: &AppShell, cx: &App) -> bool {
        match self {
            ModalKind::AuthPrompt => shell.state.modals.connection.auth_prompt.is_some(),
            ModalKind::HostKeyConfirm => shell.state.modals.connection.host_key_confirm.is_some(),
            ModalKind::ClipboardPrompt => shell.state.modals.connection.clipboard_request.is_some(),
            ModalKind::ForwardConfirm => shell.state.modals.tools.confirm_forward_any.is_some(),
            ModalKind::SftpConfirm => shell.state.sftp_modals.confirm.is_some(),
            ModalKind::SftpInput => shell.state.sftp_modals.input.is_some(),
            ModalKind::SftpChmod => shell.state.sftp_modals.chmod.is_some(),
            ModalKind::SftpProperties => shell.state.sftp_modals.properties.is_some(),
            ModalKind::SftpFilter => shell.state.sftp_modals.filter.is_some(),
            ModalKind::SftpSearch => shell.state.sftp_modals.search.is_some(),
            ModalKind::SftpSync => shell.state.sftp_modals.sync.is_some(),
            ModalKind::EditorGotoLine => shell.state.sftp_modals.goto_line.is_some(),
            ModalKind::SessionForm => shell.state.modals.session.form.is_some(),
            ModalKind::FolderForm => shell.state.modals.session.folder_form.is_some(),
            ModalKind::BatchEdit => shell.state.modals.session.batch_edit.is_some(),
            ModalKind::SnippetForm => shell.state.modals.tools.snippet_panel.form.is_some(),
            ModalKind::KnownHosts => shell.state.modals.connection.known_hosts.open,
            ModalKind::KeyWizard => shell.state.modals.tools.key_wizard.is_some(),
            ModalKind::BackupPrompt => shell.state.modals.tools.backup_prompt.is_some(),
            ModalKind::AuditViewer => shell.state.modals.tools.audit_viewer.is_some(),
            ModalKind::Replay => shell.state.modals.tools.replay.is_some(),
            ModalKind::Settings => shell.state.modals.settings.read(cx).is_open(),
            ModalKind::ShortcutCapture => shell.state.shortcut_recording.capture.is_some(),
            ModalKind::DiffViewer => shell.state.diff_viewer.is_some(),
        }
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
        match self {
            ModalKind::SessionForm => shell
                .state
                .modals
                .session
                .form
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            ModalKind::FolderForm => shell
                .state
                .modals
                .session
                .folder_form
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            ModalKind::BatchEdit => shell
                .state
                .modals
                .session
                .batch_edit
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            ModalKind::SnippetForm => shell
                .state
                .modals
                .tools
                .snippet_panel
                .form
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            // 设置页的控件是开关、下拉与滑块，不是文本框，逐控件加脏标记要改
            // 遍所有设置页且新增控件必漏；这里直接比整份设置的序列化快照，
            // 覆盖全字段。序列化失败按"不脏"处理：宁可少拦一次，也不能让
            // Esc 永远关不掉弹窗。
            ModalKind::Settings => serde_json::to_string(&app_settings(cx))
                .map(|signature| vec![signature])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
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
            shell.state.dialog_hint = Some(rust_i18n::t!("dialog.dirty_block_hint").to_string());
            cx.notify();
            return false;
        }
        shell.state.dialog_hint = None;
        match self {
            ModalKind::SftpConfirm => {
                shell.state.sftp_modals.confirm = None;
                shell.focus_active_sftp_panel(window, cx);
                true
            }
            ModalKind::HostKeyConfirm => {
                shell.reject_host_key(cx);
                true
            }
            ModalKind::ClipboardPrompt => {
                shell.deny_clipboard_write(cx);
                true
            }
            ModalKind::ForwardConfirm => {
                shell.cancel_forward_confirm(cx);
                true
            }
            ModalKind::AuthPrompt => {
                shell.state.cancel_auth_prompt();
                true
            }
            ModalKind::SftpInput => shell.close_sftp_modal(|hub| hub.input = None, window, cx),
            ModalKind::SftpChmod => shell.close_sftp_modal(|hub| hub.chmod = None, window, cx),
            ModalKind::SftpProperties => {
                shell.close_sftp_modal(|hub| hub.properties = None, window, cx)
            }
            ModalKind::SftpFilter => shell.close_sftp_modal(|hub| hub.filter = None, window, cx),
            ModalKind::SftpSearch => shell.close_sftp_modal(|hub| hub.search = None, window, cx),
            ModalKind::SftpSync => shell.close_sftp_modal(|hub| hub.sync = None, window, cx),
            ModalKind::EditorGotoLine => {
                shell.cancel_editor_goto_line(window, cx);
                true
            }
            ModalKind::SessionForm => {
                shell.state.modals.session.form = None;
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
                shell.state.modals.connection.known_hosts.open = false;
                true
            }
            ModalKind::KeyWizard => {
                shell.state.modals.tools.key_wizard = None;
                true
            }
            ModalKind::BackupPrompt => {
                shell.state.modals.tools.backup_prompt = None;
                true
            }
            ModalKind::AuditViewer => {
                shell.state.modals.tools.audit_viewer = None;
                true
            }
            ModalKind::Replay => {
                shell.state.modals.tools.replay = None;
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
                shell.close_diff_viewer(cx);
                true
            }
        }
    }

    /// 提交：等价于点主按钮。
    ///
    /// 各领域方法自己负责把状态置回 `None`（提交成功）或保留弹窗并写错误：
    /// 关闭与否一律交给对账器按期望栈收敛，不让容器先弹栈，否则校验失败时
    /// 状态仍在、下一帧又按状态把弹窗重开一次。
    /// 提交：执行主按钮动作，然后按"该弹窗状态是否已消失"决定是否允许关闭。
    ///
    /// 为什么用状态反查而不是让每个领域返回布尔：校验失败、保存失败、需要继续
    /// 填的表单都会把状态留在原处，动作函数本身并不统一返回成败；而"状态还在"
    /// 恰恰就是弹窗该继续存在的唯一事实。
    pub(super) fn commit(
        self,
        shell: &mut AppShell,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool {
        self.run_commit(shell, window, cx);
        !self.is_open(shell, cx)
    }

    fn run_commit(self, shell: &mut AppShell, window: &mut Window, cx: &mut Context<AppShell>) {
        match self {
            ModalKind::SftpConfirm => {
                shell.sftp_confirm_delete(cx);
                shell.focus_active_sftp_panel(window, cx);
            }
            ModalKind::HostKeyConfirm => shell.accept_host_key(cx),
            ModalKind::ClipboardPrompt => shell.confirm_clipboard_write(cx),
            ModalKind::ForwardConfirm => shell.confirm_forward_start(cx),
            ModalKind::AuthPrompt => shell.submit_auth_prompt(cx),
            ModalKind::SftpInput => {
                shell.commit_sftp_input_modal(window, cx);
                shell.focus_active_sftp_panel(window, cx);
            }
            ModalKind::SftpChmod => shell.save_sftp_chmod_modal(window, cx),
            // 属性与搜索弹窗只有"关闭"这一个主按钮，Enter 与它同义。
            ModalKind::SftpProperties => {
                shell.close_sftp_modal(|hub| hub.properties = None, window, cx);
            }
            ModalKind::SftpSearch => {
                shell.close_sftp_modal(|hub| hub.search = None, window, cx);
            }
            ModalKind::SftpFilter => shell.save_sftp_filter_modal(window, cx),
            ModalKind::SftpSync => shell.start_sftp_sync(window, cx),
            ModalKind::EditorGotoLine => shell.commit_editor_goto_line(window, cx),
            ModalKind::SessionForm => shell.save_session_form(cx),
            ModalKind::FolderForm => shell.save_folder_form(cx),
            ModalKind::BatchEdit => shell.save_batch_edit(cx),
            ModalKind::SnippetForm => shell.save_snippet_form(cx),
            ModalKind::BackupPrompt => shell.submit_backup(window, cx),
            // 查看与管理类弹窗只有一个"关闭"主按钮，Enter 与它同义。
            ModalKind::KnownHosts => shell.state.modals.connection.known_hosts.open = false,
            ModalKind::KeyWizard => shell.state.modals.tools.key_wizard = None,
            ModalKind::AuditViewer => shell.state.modals.tools.audit_viewer = None,
            ModalKind::Replay => shell.state.modals.tools.replay = None,
            ModalKind::Settings => shell.save_settings(window, cx),
            ModalKind::ShortcutCapture => shell.state.shortcut_recording.capture = None,
            ModalKind::DiffViewer => shell.close_diff_viewer(cx),
        }
    }

    /// 首帧焦点：文本类弹窗把焦点交给自己的输入控件，选择类交给外壳。
    ///
    /// 一律走 `as_ref()` 安全取值：关闭动作可能在对账把弹窗弹出栈之前就把状态
    /// 清空，这一帧仍可能渲染一次；用 `expect` 会把这条正常路径变成 panic。
    pub(super) fn focus_target(self, shell: &AppShell) -> ModalFocus {
        match self {
            ModalKind::SftpInput => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .input
                    .as_ref()
                    .map(|modal| modal.input.clone())
            }),
            ModalKind::SftpChmod => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .chmod
                    .as_ref()
                    .map(|modal| modal.octal_input.clone())
            }),
            ModalKind::SftpSearch => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .search
                    .as_ref()
                    .map(|modal| modal.input.clone())
            }),
            ModalKind::EditorGotoLine => input_focus(shell, |state| {
                state
                    .sftp_modals
                    .goto_line
                    .as_ref()
                    .map(|modal| modal.input.clone())
            }),
            ModalKind::SessionForm => input_focus(shell, |state| {
                state
                    .modals
                    .session
                    .form
                    .as_ref()
                    .map(|form| form.name.clone())
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
                    ModalFocus::Textarea(modal.textarea.clone())
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
