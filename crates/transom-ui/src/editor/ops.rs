//! SFTP 编辑面板的保存回传生命周期：整份缓冲区写回远端、版本指纹冲突的
//! 拦截与回填、以及结果事件的落地。
//!
//! 打开与读取在 `editor_open.rs`，面板状态与窗体动作在 `editor_panel.rs`，
//! 关闭保护在 `editor_close.rs`，渲染在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 回传走 `SftpOperation::WriteText` 独立通道，不进传输队列，理由同读取侧
//!   （见 `editor_open.rs` 模块注释）。
//! - 保存请求携带打开或上次回传成功时的版本指纹（乐观锁）：远端已被他人
//!   修改时 net 层拒绝覆盖并回报 `SftpTextConflict`，这里只置冲突横幅交用户
//!   决策（重新载入 / 强制覆盖 / 继续编辑），杜绝静默丢失他人改动。

use super::open::parent_dir;
use super::panel::{EditorPanelState, EditorPhase};
use crate::app::AppShell;
use crate::keymap;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{
    ConnectionId, ErrorCode, RemoteTextMeta, SftpCommand, SftpOperation, SudoCapability,
    SudoUnavailable, TabId, TransferId,
};

use crate::sftp::{SftpInputModalParams, SftpInputTarget};

/// 编辑回传通道：普通写回或经 sudo 提权写回。
///
/// 两条通道的准入闸门、载荷来源与投递失败兜底完全一致，差异只在命令变体
/// 与成功后清理的状态位；用一个枚举把它们收进同一投递路径，避免两份生命周期
/// 各自演化后出现口径漂移。
enum EditorWriteMode {
    /// 普通通道；`force` 为冲突横幅上显式触发的强制覆盖。
    Normal { force: bool },
    /// sudo 通道；`password` 为口令，`None` 表示探测确认可免密。
    Sudo { password: Option<String> },
}

impl AppShell {
    /// 提权保存入口：先探测远端能力，再决定直接提交还是弹口令框。
    ///
    /// 手动按钮与"权限拒绝后自动重试"共用这一入口，保证两条路径的弹窗策略一致：
    /// 免密场景完全不问，必须口令时才让用户输入。
    pub(crate) fn prompt_sudo_save(
        &mut self,
        editor: &Entity<EditorState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_sudo_probe(editor, cx);
    }

    /// 向远端发起提权能力探测；结果由 [`Self::on_sudo_probed`] 回填。
    fn request_sudo_probe(&mut self, editor: &Entity<EditorState>, cx: &mut Context<Self>) {
        let Some(index) = self
            .editors
            .panels
            .iter()
            .position(|item| item.editor == *editor)
        else {
            return;
        };
        let item = &mut self.editors.panels[index];
        if !item.loaded
            || item.pending_sudo_probe.is_some()
            || matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing)
        {
            return;
        }
        let tab_id = item.tab_id;
        // 提权探测同样要回到面板所属连接：并列远端的文件在主连接上不存在。
        let connection_id = item.connection_id;
        let op_id = TransferId::new();
        item.pending_sudo_probe = Some(op_id);
        item.sudo_probe_attempted = true;
        // 复用忙碌态阻止保存与关闭；状态栏按 pending_sudo_probe 显示"探测提权中"。
        item.phase = EditorPhase::Writing;
        item.error = Some(rust_i18n::t!("editor.status_probing_sudo").to_string());
        if !self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ProbeSudo,
        }) {
            let item = &mut self.editors.panels[index];
            item.pending_sudo_probe = None;
            item.phase = EditorPhase::Failed;
            item.error = Some(rust_i18n::t!("editor.error_probe_dispatch").to_string());
        }
        cx.notify();
    }

    /// 提权探测结果：免密直接写、需要口令才弹框、不可用则说明原因。
    pub(crate) fn on_sudo_probed(
        &mut self,
        op_id: TransferId,
        capability: SudoCapability,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editors
            .panels
            .iter()
            .position(|item| item.pending_sudo_probe == Some(op_id))
        else {
            return;
        };
        let editor = self.editors.panels[index].editor.clone();
        self.editors.panels[index].pending_sudo_probe = None;
        match capability {
            SudoCapability::Passwordless => {
                self.editors.panels[index].phase = EditorPhase::Idle;
                self.editors.panels[index].error = None;
                self.submit_sudo_editor_write(&editor, None, window, cx);
            }
            SudoCapability::PasswordRequired => {
                self.editors.panels[index].phase = EditorPhase::Idle;
                self.editors.panels[index].error = None;
                self.open_sudo_password_modal(&editor, window, cx);
            }
            SudoCapability::Unavailable { reason, detail } => {
                self.editors.panels[index].phase = EditorPhase::Failed;
                self.editors.panels[index].error = Some(
                    rust_i18n::t!(
                        "editor.sudo_unavailable",
                        reason = sudo_unavailable_text(reason),
                        detail = detail
                    )
                    .to_string(),
                );
                cx.notify();
            }
        }
    }

    /// 打开 Sudo 口令输入框；只在探测确认"必须口令"或票据失效后失败时调用。
    fn open_sudo_password_modal(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        let tab_id = item.tab_id;
        let file_name = item.file_name.clone();
        item.confirm = None;
        item.pending_sudo_probe = None;
        item.error = None;
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id,
                connection_id: None,
                title: rust_i18n::t!("editor.sudo_save").to_string(),
                prompt: rust_i18n::t!("editor.sudo_save_prompt", file_name = file_name).to_string(),
                placeholder: rust_i18n::t!("editor.sudo_password_hint").to_string(),
                value: String::new(),
                target: SftpInputTarget::SudoElevation {
                    editor: editor.clone(),
                },
            },
            window,
            cx,
        );
    }

    /// 保存：把整份缓冲区连同版本指纹交给独立写回通道，面板保持打开等待结果。
    pub(crate) fn save_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_editor_write(editor, false, window, cx);
    }

    /// 冲突横幅上的"强制覆盖远端"：跳过指纹比对直接提交当前缓冲区。
    ///
    /// 只在用户于冲突横幅上明确选择后调用；普通保存路径永远携带指纹。
    pub(crate) fn overwrite_editor_panel_after_conflict(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_editor_write(editor, true, window, cx);
    }

    /// 关闭确认横幅上的"保存并关闭"：先回传，成功即关闭面板。
    ///
    /// 失败路径（连接丢失、投递失败、回传失败、冲突）都会清掉
    /// `close_after_write`，面板留在原地显示错误，不会静默关闭。
    pub(crate) fn save_and_close_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.editor_panel_mut(editor) {
            item.close_after_write = true;
            item.confirm = None;
        }
        self.submit_editor_write(editor, false, window, cx);
    }

    /// 回传提交的共用路径：`force` 为 true 时跳过版本指纹比对（强制覆盖）。
    fn submit_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_editor_write(editor, EditorWriteMode::Normal { force }, window, cx);
    }

    /// 普通写回与提权写回的统一投递：守卫、载荷装配、发送与在途登记。
    ///
    /// 设计原理（Why）：两条通道都从同一面板快照取目标与内容，发送失败都回到
    /// 失败态并取消"保存并关闭"意图；集中一处后，新增通道只需扩展载荷分支，
    /// 不会漏掉任一状态清理。
    fn dispatch_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        mode: EditorWriteMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editors
            .panels
            .iter()
            .find(|item| item.editor == *editor)
        else {
            return;
        };
        // 没装载过远端内容（读取失败）时基线为空，写回就是拿空内容覆盖远端文件；
        // 读取在途与回传在途同理不接受新提交。三种情况一律拒绝。
        if !item.loaded || matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing) {
            return;
        }
        let tab_id = item.tab_id;
        let connection_id = item.connection_id;
        let remote_path = item.remote_path.clone();
        // expected 始终携带已知指纹：它同时承载并发比对基准与 BOM/CRLF 形态还原
        // 标记（强制覆盖也必须还原形态，否则保存一次就改掉文件行尾）；跳过比对
        // 由 force 表达。新文件面板指纹为 None，net 层据此不做比对与还原。
        let expected = item.remote_meta;
        let content = editor.read(cx).value().as_bytes().to_vec();
        let backup = self.settings.sftp.editor_write_backup;
        let (sudo, force) = match &mode {
            EditorWriteMode::Normal { force } => (false, *force),
            EditorWriteMode::Sudo { .. } => (true, false),
        };
        let op = match mode {
            EditorWriteMode::Normal { force } => SftpOperation::WriteText {
                path: remote_path,
                content,
                expected,
                force,
                backup,
            },
            EditorWriteMode::Sudo { password } => SftpOperation::SudoWriteText {
                path: remote_path,
                content,
                expected,
                // 普通提权保存仍受版本指纹保护；强制覆盖只由冲突横幅显式触发，
                // 提权通道不提供绕过入口。
                force: false,
                backup,
                password,
            },
        };

        let op_id = TransferId::new();
        if let Err(error) = self.services.command_tx.try_send(
            SftpCommand::Sftp {
                tab_id,
                connection_id,
                op_id,
                op,
            }
            .into(),
        ) {
            if let Some(item) = self.editor_panel_mut(editor) {
                item.phase = EditorPhase::Failed;
                item.close_after_write = false;
                item.error = Some(crate::app::command_send_error_text(error));
            }
            self.focus_active_sftp_panel(window, cx);
            cx.notify();
            return;
        }
        // 在途 ID 先落到面板：完成事件与本次调用同处一个事件循环，回来时归属已确定。
        if let Some(item) = self.editor_panel_mut(editor) {
            item.pending_op = Some(op_id);
            item.phase = EditorPhase::Writing;
            item.error = None;
            item.confirm = None;
            if sudo {
                item.pending_sudo_probe = None;
            } else if force {
                item.conflict = false;
            }
        }
        cx.notify();
    }

    /// 执行 Sudo 提权写入提交：通过独立 Sudo 通道以标准流灌入并原子替换。
    pub(crate) fn submit_sudo_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_editor_write(editor, EditorWriteMode::Sudo { password }, window, cx);
    }

    /// 回传成功：脏基准与版本指纹都换成远端确认后的新值，目录列表条目就地
    /// 补丁；请求过"保存并关闭"时在此关闭面板。
    ///
    /// 保存成功即脏面板清零，可能因此满足挂起的标签关闭，续行检查。
    pub(crate) fn note_editor_text_written(
        &mut self,
        op_id: TransferId,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editors
            .panels
            .iter()
            .position(|item| item.pending_op == Some(op_id))
        else {
            return;
        };
        let (editor, close_after, was_new_file, tab_id, remote_path) = {
            let item = &mut self.editors.panels[index];
            item.pending_op = None;
            item.baseline = item.editor.read(cx).value();
            item.remote_meta = Some(meta);
            item.dirty = false;
            item.phase = EditorPhase::Written;
            item.written_size = Some(meta.size);
            item.error = None;
            item.conflict = false;
            // 差异预览展示的"待回传变更"已经落盘，回编辑视图展示新基线内容。
            item.showing_diff = false;
            item.diff = None;
            let close_after = item.close_after_write;
            item.close_after_write = false;
            let was_new_file = item.new_file;
            // 首次保存成功，面板自此对应一个真实存在的远端文件。
            item.new_file = false;
            (
                item.editor.clone(),
                close_after,
                was_new_file,
                item.tab_id,
                item.remote_path.clone(),
            )
        };
        // 目录列表条目就地补丁：size/mtime 都来自写后 stat，与远端一致，
        // 无需为刷新列表再发一次目录请求。
        if let Some(tab) = self.workspace.tab_mut(tab_id)
            && let Some(panel) = tab.sftp_mut()
            && let Some(entry) = panel
                .remote
                .entries
                .iter_mut()
                .find(|entry| entry.path == remote_path)
        {
            entry.size = meta.size;
            entry.modified = meta.mtime;
        }
        // 新建文件首次落盘：列表里还没有这个条目，就地补丁无效，按需刷新
        // 当前目录（仅当用户仍停留在该文件所在目录时）。
        if was_new_file
            && let Some(parent) = parent_dir(&remote_path)
            && self
                .workspace
                .tab(tab_id)
                .and_then(|tab| tab.sftp())
                .is_some_and(|panel| panel.remote.path == parent)
        {
            self.request_sftp_list(tab_id, parent.to_string(), cx);
        }
        if close_after {
            self.drop_editor_panel(&editor, window, cx);
            self.resume_pending_editor_close(window, cx);
            return;
        }
        cx.notify();
        self.resume_pending_editor_close(window, cx);
    }

    /// 回传被版本指纹拦截：远端已被修改，置冲突横幅交用户决策，内容原样保留。
    ///
    /// `dirty` 不动：用户继续点保存会再次命中冲突，出口只有横幅上的三个动作。
    /// "保存并关闭"意图一并取消——关闭动作必须等用户在冲突横幅上做出选择。
    pub(crate) fn note_editor_text_conflict(
        &mut self,
        op_id: TransferId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editors
            .panels
            .iter_mut()
            .find(|item| item.pending_op == Some(op_id))
        else {
            return;
        };
        item.pending_op = None;
        item.phase = EditorPhase::Failed;
        item.written_size = None;
        item.conflict = true;
        item.confirm = None;
        item.close_after_write = false;
        item.error =
            Some(rust_i18n::t!("editor.error_remote_changed", message = message).to_string());
        cx.notify();
    }

    /// 读取或回传失败：错误就地显示；读取失败面板保持空缓冲区，回传失败内容原样保留。
    ///
    /// 写回失败且错误码是权限拒绝时，自动转入提权探测而不是只留一条错误文本：
    /// 这是唯一有明确恢复路径的失败。探测只自动走一次，避免提权仍被拒时形成循环；
    /// sudo 票据失效（`AuthFailed`）则直接重新询问口令。
    pub(crate) fn note_editor_text_failed(
        &mut self,
        op_id: TransferId,
        message: String,
        code: ErrorCode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.editors.panels.iter().position(|item| {
            item.pending_op == Some(op_id) || item.pending_sudo_probe == Some(op_id)
        }) else {
            return;
        };
        let item = &mut self.editors.panels[index];
        if item.pending_sudo_probe == Some(op_id) {
            // 探测失败：没有可继续的提权结论，直接回到失败态并保留远端原文。
            item.pending_sudo_probe = None;
            item.phase = EditorPhase::Failed;
            item.error = Some(
                rust_i18n::t!("editor.error_sudo_probe_failed", message = message).to_string(),
            );
            cx.notify();
            return;
        }
        let was_writing = item.phase == EditorPhase::Writing;
        let detail = if item.phase == EditorPhase::Reading {
            rust_i18n::t!("editor.error_read_failed", message = message).to_string()
        } else {
            rust_i18n::t!("editor.error_write_failed", message = message).to_string()
        };
        let editor = item.editor.clone();
        let should_probe =
            was_writing && code == ErrorCode::PermissionDenied && !item.sudo_probe_attempted;
        let should_reask = was_writing && code == ErrorCode::AuthFailed;
        item.pending_op = None;
        item.phase = EditorPhase::Failed;
        item.written_size = None;
        item.close_after_write = false;
        item.error = Some(detail);
        if should_probe {
            item.error = Some(rust_i18n::t!("editor.status_probing_sudo_after_denied").to_string());
            self.request_sudo_probe(&editor, cx);
        } else if should_reask {
            item.phase = EditorPhase::Idle;
            item.error = None;
            self.open_sudo_password_modal(&editor, window, cx);
            return;
        }
        cx.notify();
    }

    /// 命令投递失败这类本地错误的兜底：按在途 ID 定位面板并置失败态。
    pub(crate) fn fail_editor_panel_by_op(
        &mut self,
        op_id: TransferId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .editors
            .panels
            .iter_mut()
            .find(|item| item.pending_op == Some(op_id))
        {
            item.pending_op = None;
            item.phase = EditorPhase::Failed;
            item.error = Some(message);
        }
        cx.notify();
    }

    /// 把编辑入口的拒绝原因写到目标远端列的错误位上。
    pub(crate) fn editor_reject(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.workspace.tab_mut(tab_id) {
            let main_connection = tab.connection_id;
            if let Some(panel) = tab.sftp_mut()
                && let Some(target) =
                    panel.remote_pane_of_connection(connection_id, main_connection)
                && let Some(pane) = panel.remote_pane_mut(target)
            {
                pane.error = Some(message);
            }
        }
        cx.notify();
    }

    /// 面板内保存快捷键：焦点在哪个面板就存哪个，取不到焦点时存最上层，
    /// 避免键位静默失效让用户以为没按到。
    pub(crate) fn on_save_editor_panel(
        &mut self,
        _: &keymap::SaveEditorFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.focused_editor_panel_index(window, cx) else {
            return;
        };
        let Some(editor) = self
            .editors
            .panels
            .get(index)
            .map(|item| item.editor.clone())
        else {
            return;
        };
        self.save_editor_panel(&editor, window, cx);
    }

    /// 当前应接收"保存"快捷键的面板下标：优先编辑器持有焦点的那个，
    /// 没有焦点信息时退到最上层，避免快捷键静默失效。
    pub(crate) fn focused_editor_panel_index(&self, window: &Window, cx: &App) -> Option<usize> {
        if self.editors.panels.is_empty() {
            return None;
        }
        self.editors
            .panels
            .iter()
            .position(|item| {
                item.editor
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            })
            .or(Some(self.editors.panels.len() - 1))
    }

    pub(crate) fn editor_panel_mut(
        &mut self,
        editor: &Entity<EditorState>,
    ) -> Option<&mut EditorPanelState> {
        self.editors
            .panels
            .iter_mut()
            .find(|item| item.editor == *editor)
    }

    pub(crate) fn drop_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editors.panels.retain(|item| item.editor != *editor);
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }
}

/// 把提权不可用原因翻译成用户能读懂、可据以采取行动的说明。
fn sudo_unavailable_text(reason: SudoUnavailable) -> String {
    match reason {
        SudoUnavailable::NotInstalled => rust_i18n::t!("editor.sudo_not_installed").to_string(),
        SudoUnavailable::RequiresTty => rust_i18n::t!("editor.sudo_requires_tty").to_string(),
        SudoUnavailable::NotAuthorized => rust_i18n::t!("editor.sudo_not_authorized").to_string(),
        SudoUnavailable::Other => rust_i18n::t!("editor.sudo_probe_failed_reason").to_string(),
    }
}
