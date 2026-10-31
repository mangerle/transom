//! 编辑器保存、提权探测与写回投递。

use super::*;

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
    pub(super) fn request_sudo_probe(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .state
            .editors
            .panels
            .iter()
            .position(|item| item.editor == *editor)
        else {
            return;
        };
        let item = &mut self.state.editors.panels[index];
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
        if !self.state.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ProbeSudo,
        }) {
            let item = &mut self.state.editors.panels[index];
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
            .state
            .editors
            .panels
            .iter()
            .position(|item| item.pending_sudo_probe == Some(op_id))
        else {
            return;
        };
        let editor = self.state.editors.panels[index].editor.clone();
        self.state.editors.panels[index].pending_sudo_probe = None;
        match capability {
            SudoCapability::Passwordless => {
                self.state.editors.panels[index].phase = EditorPhase::Idle;
                self.state.editors.panels[index].error = None;
                self.submit_sudo_editor_write(&editor, None, window, cx);
            }
            SudoCapability::PasswordRequired => {
                self.state.editors.panels[index].phase = EditorPhase::Idle;
                self.state.editors.panels[index].error = None;
                self.open_sudo_password_modal(&editor, window, cx);
            }
            SudoCapability::Unavailable { reason, detail } => {
                self.state.editors.panels[index].phase = EditorPhase::Failed;
                self.state.editors.panels[index].error = Some(
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
    pub(super) fn open_sudo_password_modal(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((tab_id, file_name)) = self.map_editor_panel(editor, |item| {
            let tab_id = item.tab_id;
            let file_name = item.file_name.clone();
            item.confirm = None;
            item.pending_sudo_probe = None;
            item.error = None;
            (tab_id, file_name)
        }) else {
            return;
        };
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
        self.update_editor_panel(editor, |item| {
            item.close_after_write = true;
            item.confirm = None;
        });
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
            .state
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
        let backup = self.state.settings.sftp.editor_write_backup;
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
        if let Err(error) = self.state.services.command_tx.try_send(
            SftpCommand::Sftp {
                tab_id,
                connection_id,
                op_id,
                op,
            }
            .into(),
        ) {
            self.update_editor_panel(editor, |item| {
                item.phase = EditorPhase::Failed;
                item.close_after_write = false;
                item.error = Some(crate::app::command_send_error_text(error));
            });
            self.focus_active_sftp_panel(window, cx);
            cx.notify();
            return;
        }
        // 在途 ID 先落到面板：完成事件与本次调用同处一个事件循环，回来时归属已确定。
        self.update_editor_panel(editor, |item| {
            item.pending_op = Some(op_id);
            item.phase = EditorPhase::Writing;
            item.error = None;
            item.confirm = None;
            if sudo {
                item.pending_sudo_probe = None;
            } else if force {
                item.conflict = false;
            }
        });
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
}
