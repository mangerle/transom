//! 编辑器写回结果、冲突与失败处理。

use super::*;

impl AppShell {
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
