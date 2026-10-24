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

use super::editor_panel::{EditorPanelState, EditorPhase};
use crate::app::AppShell;
use crate::keymap;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{AppCommand, RemoteTextMeta, SftpOperation, TabId, TransferId};

impl AppShell {
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

    /// 回传提交的共用路径：`force` 为 true 时跳过版本指纹比对（强制覆盖）。
    fn submit_editor_write(
        &mut self,
        editor: &Entity<EditorState>,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editor_panels
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
        let remote_path = item.remote_path.clone();
        let expected = if force { None } else { item.remote_meta };
        let content = editor.read(cx).value();
        let Some(connection_id) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.connection_id)
        else {
            if let Some(item) = self.editor_panel_mut(editor) {
                item.phase = EditorPhase::Failed;
                item.error =
                    Some("所属标签的连接已断开，无法回传，请复制内容后重新打开".to_string());
            }
            self.focus_active_sftp_panel(window, cx);
            cx.notify();
            return;
        };

        let op_id = TransferId::new();
        if !self.send_command(AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::WriteText {
                path: remote_path,
                content: content.as_bytes().to_vec(),
                expected,
            },
        }) {
            if let Some(item) = self.editor_panel_mut(editor) {
                item.phase = EditorPhase::Failed;
                item.error = Some("回传命令投递失败".to_string());
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
            if force {
                item.conflict = false;
            }
        }
        cx.notify();
    }

    /// 回传成功：脏基准与版本指纹都换成远端确认后的新值。
    ///
    /// 保存成功即脏面板清零，可能因此满足挂起的标签关闭，续行检查。
    pub(crate) fn note_editor_text_written(
        &mut self,
        op_id: TransferId,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editor_panels
            .iter_mut()
            .find(|item| item.pending_op == Some(op_id))
        else {
            return;
        };
        item.pending_op = None;
        item.baseline = item.editor.read(cx).value();
        item.remote_meta = Some(meta);
        item.dirty = false;
        item.phase = EditorPhase::Written;
        item.written_size = Some(meta.size);
        item.error = None;
        item.conflict = false;
        cx.notify();
        self.resume_pending_editor_close(window, cx);
    }

    /// 回传被版本指纹拦截：远端已被修改，置冲突横幅交用户决策，内容原样保留。
    ///
    /// `dirty` 不动：用户继续点保存会再次命中冲突，出口只有横幅上的三个动作。
    pub(crate) fn note_editor_text_conflict(
        &mut self,
        op_id: TransferId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editor_panels
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
        item.error = Some(format!("远端已被修改: {message}"));
        cx.notify();
    }

    /// 读取或回传失败：错误就地显示；读取失败面板保持空缓冲区，回传失败内容原样保留。
    pub(crate) fn note_editor_text_failed(
        &mut self,
        op_id: TransferId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editor_panels
            .iter_mut()
            .find(|item| item.pending_op == Some(op_id))
        else {
            return;
        };
        let (action, detail) = if item.phase == EditorPhase::Reading {
            ("读取远端失败", message)
        } else {
            ("回传远端失败", message)
        };
        item.pending_op = None;
        item.phase = EditorPhase::Failed;
        item.written_size = None;
        item.error = Some(format!("{action}: {detail}"));
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
            .editor_panels
            .iter_mut()
            .find(|item| item.pending_op == Some(op_id))
        {
            item.pending_op = None;
            item.phase = EditorPhase::Failed;
            item.error = Some(message);
        }
        cx.notify();
    }

    /// 把编辑入口的拒绝原因写到该标签的远端栏错误位上。
    pub(crate) fn editor_reject(&mut self, tab_id: TabId, message: String, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && let Some(panel) = tab.sftp.as_mut()
        {
            panel.error = Some(message);
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
            .editor_panels
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
        if self.editor_panels.is_empty() {
            return None;
        }
        self.editor_panels
            .iter()
            .position(|item| {
                item.editor
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            })
            .or(Some(self.editor_panels.len() - 1))
    }

    pub(crate) fn editor_panel_mut(
        &mut self,
        editor: &Entity<EditorState>,
    ) -> Option<&mut EditorPanelState> {
        self.editor_panels
            .iter_mut()
            .find(|item| item.editor == *editor)
    }

    pub(crate) fn drop_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor_panels.retain(|item| item.editor != *editor);
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }
}
