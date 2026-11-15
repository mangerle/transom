//! 编辑器写回结果、提权探测结果与失败处理。
//!
//! 两段式：实体落到面板状态并返回决策（回传成功快照、探测请求、口令弹窗
//! 快照），外壳按决策发命令/开弹窗；外壳方法不读写本实体。

use super::super::close::PendingEditorClose;
use super::save::{SudoPasswordRequest, SudoProbeRequest};
use super::*;
use transom_core::RemoteTextMeta;

/// 回传成功后交给外壳的跨域动作快照。
pub(in crate::editor) struct EditorWrittenAction {
    pub(in crate::editor) tab_id: TabId,
    pub(in crate::editor) remote_path: String,
    pub(in crate::editor) meta: RemoteTextMeta,
    /// 新建文件首次落盘：列表里还没有该条目，需要按需刷新当前目录。
    pub(in crate::editor) was_new_file: bool,
    /// 面板因"保存并关闭"已从集合移除，外壳需要把焦点交还 SFTP。
    pub(in crate::editor) dropped: bool,
    /// 挂起的标签关闭意图快照；由外壳结合受影响标签判定能否续行。
    pub(in crate::editor) pending_close: Option<PendingEditorClose>,
    /// 当前仍为脏的标签快照。
    pub(in crate::editor) dirty_tabs: Vec<TabId>,
}

/// 提权探测结果落地后的下一步。
pub(in crate::editor) enum SudoProbeOutcome {
    /// 免密可用：直接提交提权写入。
    WritePasswordless { editor: Entity<EditorState> },
    /// 必须口令：外壳打开口令输入框。
    AskPassword(SudoPasswordRequest),
    /// 已落到失败态，无后续动作。
    None,
}

/// 读取/回传失败落地后的下一步。
pub(in crate::editor) enum EditorFailureAction {
    /// 权限拒绝且尚未自动探测过：外壳投递提权探测。
    Probe(SudoProbeRequest),
    /// sudo 票据失效：外壳重新询问口令。
    AskPassword(SudoPasswordRequest),
    /// 已落到失败态，无后续动作。
    None,
}

impl EditorPanels {
    /// 回传成功：脏基准与版本指纹都换成远端确认后的新值，目录列表条目由外壳
    /// 就地补丁；请求过"保存并关闭"时在这里移除面板。
    pub(in crate::editor) fn note_editor_text_written(
        &mut self,
        op_id: TransferId,
        meta: RemoteTextMeta,
        cx: &mut Context<Self>,
    ) -> Option<EditorWrittenAction> {
        let index = self
            .manager()
            .panels
            .iter()
            .position(|item| item.pending_op == Some(op_id))?;
        let (editor, close_after, was_new_file, tab_id, remote_path) = {
            let item = &mut self.manager_mut().panels[index];
            item.pending_op = None;
            item.baseline = item.editor.read(cx).value();
            item.remote_meta = Some(meta);
            item.dirty = false;
            item.phase = EditorPhase::Written;
            item.written_size = Some(meta.size);
            item.error = None;
            item.conflict = false;
            // 差异预览展示的"待回传变更"已经落盘，回编辑视图展示新基线内容。
            item.clear_diff_preview();
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
        if close_after {
            self.remove_panel(&editor);
        } else {
            self.clear_change_decorations(&editor, cx);
        }
        let pending_close = self.pending_close();
        let dirty_tabs = self.dirty_tabs();
        cx.notify();
        Some(EditorWrittenAction {
            tab_id,
            remote_path,
            meta,
            was_new_file,
            dropped: close_after,
            pending_close,
            dirty_tabs,
        })
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
            .manager_mut()
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
    pub(in crate::editor) fn note_editor_text_failed(
        &mut self,
        op_id: TransferId,
        message: String,
        code: ErrorCode,
        cx: &mut Context<Self>,
    ) -> EditorFailureAction {
        let Some(index) = self.manager().panels.iter().position(|item| {
            item.pending_op == Some(op_id) || item.pending_sudo_probe == Some(op_id)
        }) else {
            return EditorFailureAction::None;
        };
        if self.manager().panels[index].pending_sudo_probe == Some(op_id) {
            // 探测失败：没有可继续的提权结论，直接回到失败态并保留远端原文。
            let item = &mut self.manager_mut().panels[index];
            item.pending_sudo_probe = None;
            item.phase = EditorPhase::Failed;
            item.error = Some(
                rust_i18n::t!("editor.error_sudo_probe_failed", message = message).to_string(),
            );
            cx.notify();
            return EditorFailureAction::None;
        }
        let (editor, tab_id, file_name, should_probe, should_reask) = {
            let item = &mut self.manager_mut().panels[index];
            let was_writing = item.phase == EditorPhase::Writing;
            let detail = if item.phase == EditorPhase::Reading {
                rust_i18n::t!("editor.error_read_failed", message = message).to_string()
            } else {
                rust_i18n::t!("editor.error_write_failed", message = message).to_string()
            };
            let should_probe =
                was_writing && code == ErrorCode::PermissionDenied && !item.sudo_probe_attempted;
            let should_reask = was_writing && code == ErrorCode::AuthFailed;
            item.pending_op = None;
            item.phase = EditorPhase::Failed;
            item.written_size = None;
            item.close_after_write = false;
            item.error = Some(detail);
            if should_probe {
                item.error =
                    Some(rust_i18n::t!("editor.status_probing_sudo_after_denied").to_string());
            } else if should_reask {
                item.phase = EditorPhase::Idle;
                item.error = None;
            }
            (
                item.editor.clone(),
                item.tab_id,
                item.file_name.clone(),
                should_probe,
                should_reask,
            )
        };
        if should_probe {
            let request = self.prepare_sudo_probe(&editor);
            cx.notify();
            return match request {
                Some(request) => EditorFailureAction::Probe(request),
                None => EditorFailureAction::None,
            };
        }
        if should_reask {
            cx.notify();
            return EditorFailureAction::AskPassword(SudoPasswordRequest {
                editor,
                tab_id,
                file_name,
            });
        }
        cx.notify();
        EditorFailureAction::None
    }

    /// 命令投递失败这类本地错误的兜底：按在途 ID 定位面板并置失败态。
    pub(crate) fn fail_by_op(
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
            item.error = Some(message);
        }
        cx.notify();
    }

    /// 提权探测结果：免密直接写、需要口令才弹框、不可用则说明原因。
    pub(in crate::editor) fn note_sudo_probed(
        &mut self,
        op_id: TransferId,
        capability: SudoCapability,
        cx: &mut Context<Self>,
    ) -> SudoProbeOutcome {
        let Some(index) = self
            .manager()
            .panels
            .iter()
            .position(|item| item.pending_sudo_probe == Some(op_id))
        else {
            return SudoProbeOutcome::None;
        };
        let item = &mut self.manager_mut().panels[index];
        let editor = item.editor.clone();
        item.pending_sudo_probe = None;
        match capability {
            SudoCapability::Passwordless => {
                item.phase = EditorPhase::Idle;
                item.error = None;
                cx.notify();
                SudoProbeOutcome::WritePasswordless { editor }
            }
            SudoCapability::PasswordRequired => {
                item.phase = EditorPhase::Idle;
                item.error = None;
                item.confirm = None;
                let request = SudoPasswordRequest {
                    editor,
                    tab_id: item.tab_id,
                    file_name: item.file_name.clone(),
                };
                cx.notify();
                SudoProbeOutcome::AskPassword(request)
            }
            SudoCapability::Unavailable { reason, detail } => {
                item.phase = EditorPhase::Failed;
                item.error = Some(
                    rust_i18n::t!(
                        "editor.sudo_unavailable",
                        reason = sudo_unavailable_text(reason),
                        detail = detail
                    )
                    .to_string(),
                );
                cx.notify();
                SudoProbeOutcome::None
            }
        }
    }
}
