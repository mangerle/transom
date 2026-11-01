//! 编辑器保存、提权探测与写回投递。
//!
//! 实体自己持有命令通道：写回与提权探测直接在这里投递，失败时把结果落回
//! 面板状态；需要窗口的收尾动作通过事件交给外壳。

use super::*;
use crate::app::app_settings;

/// 编辑回传通道：普通写回或经 sudo 提权写回。
///
/// 两条通道的准入闸门、载荷来源与投递失败兜底完全一致，差异只在命令变体
/// 与成功后清理的状态位；用一个枚举把它们收进同一投递路径，避免两份生命周期
/// 各自演化后出现口径漂移。
pub(super) enum EditorWriteMode {
    /// 普通通道；`force` 为冲突横幅上显式触发的强制覆盖。
    Normal { force: bool },
    /// sudo 通道；`password` 为口令，`None` 表示探测确认可免密。
    Sudo { password: Option<String> },
}

/// 实体改完状态后交给外壳投递的一次写回请求。
pub(super) struct EditorWriteRequest {
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) op_id: TransferId,
    pub(super) remote_path: String,
    pub(super) content: Vec<u8>,
    pub(super) expected: Option<RemoteTextMeta>,
    pub(super) sudo: bool,
    pub(super) force: bool,
    pub(super) password: Option<String>,
}

/// 实体改完状态后交给外壳投递的一次提权探测请求。
pub(super) struct SudoProbeRequest {
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) op_id: TransferId,
}

/// 实体清完状态后交给外壳打开口令弹窗所需的数据快照。
pub(super) struct SudoPasswordRequest {
    pub(super) editor: Entity<EditorState>,
    pub(super) tab_id: TabId,
    pub(super) file_name: String,
}

impl EditorPanels {
    /// 保存：把整份缓冲区连同版本指纹交给独立写回通道，面板保持打开等待结果。
    pub(crate) fn save_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_editor_write(editor, EditorWriteMode::Normal { force: false }, window, cx);
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
        self.submit_editor_write(editor, EditorWriteMode::Normal { force: true }, window, cx);
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
        self.update_panel(editor, |item| {
            item.close_after_write = true;
            item.confirm = None;
        });
        self.submit_editor_write(editor, EditorWriteMode::Normal { force: false }, window, cx);
    }

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
        let Some(request) = self.prepare_sudo_probe(editor) else {
            return;
        };
        let op_id = request.op_id;
        let sent = self.command_tx().is_some_and(|tx| {
            tx.try_send(
                SftpCommand::Sftp {
                    tab_id: request.tab_id,
                    connection_id: request.connection_id,
                    op_id: request.op_id,
                    op: SftpOperation::ProbeSudo,
                }
                .into(),
            )
            .is_ok()
        });
        if !sent {
            self.fail_sudo_probe_dispatch(op_id, cx);
        }
        cx.notify();
    }

    /// 状态改完之后提交写回：投递失败时把错误落回面板。
    /// 面板自己投递写回命令；保留同步返回值，失败时由调用方落错误态。
    fn dispatch_write(
        &mut self,
        request: EditorWriteRequest,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let backup = app_settings(cx).sftp.editor_write_backup;
        // expected 始终携带已知指纹：它同时承载并发比对基准与 BOM/CRLF 形态还原
        // 标记（强制覆盖也必须还原形态，否则保存一次就改掉文件行尾）。
        let op = if request.sudo {
            SftpOperation::SudoWriteText {
                path: request.remote_path,
                content: request.content,
                expected: request.expected,
                force: false,
                backup,
                password: request.password,
            }
        } else {
            SftpOperation::WriteText {
                path: request.remote_path,
                content: request.content,
                expected: request.expected,
                force: request.force,
                backup,
            }
        };
        let Some(tx) = self.command_tx() else {
            return Err(rust_i18n::t!("editor.error_write_dispatch").to_string());
        };
        tx.try_send(
            SftpCommand::Sftp {
                tab_id: request.tab_id,
                connection_id: request.connection_id,
                op_id: request.op_id,
                op,
            }
            .into(),
        )
        .map_err(crate::app::command_send_error_text)?;
        cx.notify();
        Ok(())
    }

    fn submit_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        mode: EditorWriteMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.prepare_editor_write(editor, mode, cx) else {
            return;
        };
        let op_id = request.op_id;
        match self.dispatch_write(request, cx) {
            Ok(()) => {}
            Err(message) => {
                // 投递失败时把焦点还给 SFTP 面板，让用户看到错误来源。
                cx.emit(crate::editor::EditorPanelEvent::FocusSftpPanel);
                self.fail_editor_write_dispatch(op_id, message, cx);
            }
        }
    }

    /// 写回准入闸门与载荷装配；返回 `None` 表示当前不允许写回。
    fn prepare_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        mode: EditorWriteMode,
        cx: &App,
    ) -> Option<EditorWriteRequest> {
        let (tab_id, connection_id, remote_path, expected) = self
            .map_panel(editor, |item| {
                // 没装载过远端内容（读取失败）时基线为空，写回就是拿空内容覆盖远端
                // 文件；读取在途与回传在途同理不接受新提交。
                if !item.loaded || Self::phase_busy(item.phase) {
                    return None;
                }
                Some((
                    item.tab_id,
                    item.connection_id,
                    item.remote_path.clone(),
                    item.remote_meta,
                ))
            })
            .flatten()?;
        let content = editor.read(cx).value().as_bytes().to_vec();
        let (sudo, force, password) = match mode {
            EditorWriteMode::Normal { force } => (false, force, None),
            EditorWriteMode::Sudo { password } => (true, false, password),
        };
        let op_id = TransferId::new();
        self.update_panel(editor, |item| {
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
        Some(EditorWriteRequest {
            tab_id,
            connection_id,
            op_id,
            remote_path,
            content,
            expected,
            sudo,
            force,
            password,
        })
    }

    /// 提权探测准入闸门与状态登记；返回 `None` 表示当前不允许探测。
    pub(super) fn prepare_sudo_probe(
        &mut self,
        editor: &Entity<EditorState>,
    ) -> Option<SudoProbeRequest> {
        self.map_panel(editor, |item| {
            if !item.loaded || item.pending_sudo_probe.is_some() || Self::phase_busy(item.phase) {
                return None;
            }
            let op_id = TransferId::new();
            item.pending_sudo_probe = Some(op_id);
            item.sudo_probe_attempted = true;
            // 复用忙碌态阻止保存与关闭；状态栏按 pending_sudo_probe 显示"探测提权中"。
            item.phase = EditorPhase::Writing;
            item.error = Some(rust_i18n::t!("editor.status_probing_sudo").to_string());
            Some(SudoProbeRequest {
                tab_id: item.tab_id,
                connection_id: item.connection_id,
                op_id,
            })
        })
        .flatten()
    }

    /// 探测投递失败：回到失败态，保留可原地重试的错误文本。
    pub(super) fn fail_sudo_probe_dispatch(&mut self, op_id: TransferId, cx: &mut Context<Self>) {
        if let Some(item) = self
            .manager_mut()
            .panels
            .iter_mut()
            .find(|item| item.pending_sudo_probe == Some(op_id))
        {
            item.pending_sudo_probe = None;
            item.phase = EditorPhase::Failed;
            item.error = Some(rust_i18n::t!("editor.error_probe_dispatch").to_string());
        }
        cx.notify();
    }

    /// 写回投递失败：回到失败态并取消"保存并关闭"意图。
    fn fail_editor_write_dispatch(
        &mut self,
        op_id: TransferId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .manager_mut()
            .panels
            .iter_mut()
            .find(|item| item.pending_op == Some(op_id))
        {
            item.pending_op = None;
            item.phase = EditorPhase::Failed;
            item.close_after_write = false;
            item.error = Some(message);
        }
        cx.notify();
    }
}

impl AppShell {
    /// 向远端发起提权能力探测；结果由 [`Self::on_sudo_probed`] 回填。
    pub(super) fn send_sudo_probe(&mut self, request: &SudoProbeRequest) -> Result<(), ()> {
        if self.state.send_command(SftpCommand::Sftp {
            tab_id: request.tab_id,
            connection_id: request.connection_id,
            op_id: request.op_id,
            op: SftpOperation::ProbeSudo,
        }) {
            Ok(())
        } else {
            Err(())
        }
    }

    /// 普通写回与提权写回的统一投递：载荷装配、发送与在途登记已在实体完成。
    ///
    /// 设计原理（Why）：两条通道都从同一面板快照取目标与内容，发送失败都回到
    /// 失败态并取消"保存并关闭"意图；集中一处后，新增通道只需扩展载荷分支，
    /// 不会漏掉任一状态清理。
    fn dispatch_editor_write(
        &mut self,
        request: EditorWriteRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let backup = app_settings(cx).sftp.editor_write_backup;
        // expected 始终携带已知指纹：它同时承载并发比对基准与 BOM/CRLF 形态还原
        // 标记（强制覆盖也必须还原形态，否则保存一次就改掉文件行尾）；跳过比对
        // 由 force 表达。新文件面板指纹为 None，net 层据此不做比对与还原。
        let op = if request.sudo {
            SftpOperation::SudoWriteText {
                path: request.remote_path,
                content: request.content,
                expected: request.expected,
                // 普通提权保存仍受版本指纹保护；强制覆盖只由冲突横幅显式触发，
                // 提权通道不提供绕过入口。
                force: false,
                backup,
                password: request.password,
            }
        } else {
            SftpOperation::WriteText {
                path: request.remote_path,
                content: request.content,
                expected: request.expected,
                force: request.force,
                backup,
            }
        };
        let command = SftpCommand::Sftp {
            tab_id: request.tab_id,
            connection_id: request.connection_id,
            op_id: request.op_id,
            op,
        };
        if let Err(error) = self.state.services.command_tx.try_send(command.into()) {
            self.focus_active_sftp_panel(window, cx);
            return Err(crate::app::command_send_error_text(error));
        }
        cx.notify();
        Ok(())
    }

    /// 打开 Sudo 口令输入框；只在探测确认"必须口令"或票据失效后失败时调用。
    pub(super) fn open_sudo_password_modal(
        &mut self,
        request: SudoPasswordRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sftp_input_modal(
            SftpInputModalParams {
                tab_id: request.tab_id,
                connection_id: None,
                title: rust_i18n::t!("editor.sudo_save").to_string(),
                prompt: rust_i18n::t!("editor.sudo_save_prompt", file_name = request.file_name)
                    .to_string(),
                placeholder: rust_i18n::t!("editor.sudo_password_hint").to_string(),
                value: String::new(),
                target: SftpInputTarget::SudoElevation {
                    editor: request.editor,
                },
            },
            window,
            cx,
        );
    }

    /// 执行 Sudo 提权写入提交：改状态由实体完成，这里只负责投递命令。
    pub(crate) fn submit_sudo_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.state.editors.update(cx, |panels, cx| {
            panels.prepare_editor_write(editor, EditorWriteMode::Sudo { password }, cx)
        }) else {
            return;
        };
        let op_id = request.op_id;
        if let Err(message) = self.dispatch_editor_write(request, window, cx) {
            self.state.editors.update(cx, |panels, cx| {
                panels.fail_editor_write_dispatch(op_id, message, cx)
            });
        }
    }
}
