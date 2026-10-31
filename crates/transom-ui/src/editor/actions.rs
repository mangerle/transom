//! SFTP 编辑面板的窗体动作：折叠/最大化、置顶与拖拽、关闭与恢复的确认流程。
//!
//! 面板状态与几何换算在 `panel.rs`，打开与读取在 `open.rs`，
//! 保存回传在 `ops.rs`，关闭闸门在 `close.rs`，渲染在 `view.rs`。
//!
//! 这些方法都挂在 `EditorPanels` 实体上：状态变更在本实体完成，需要发命令、
//! 开弹窗或聚焦其它面板时通过 `with_shell` 回交外壳。

use super::entity::EditorPanels;
use super::panel::{EditorPhase, PanelConfirm};
use gpui_kit::component::input::EditorState;
use gpui_kit::*;

impl EditorPanels {
    pub(crate) fn toggle_editor_panel_collapsed(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        if !self.manager_mut().toggle_collapsed(editor) {
            return;
        }
        cx.notify();
    }

    pub(crate) fn toggle_editor_panel_maximized(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        if !self.manager_mut().toggle_maximized(editor) {
            return;
        }
        cx.notify();
    }

    /// 把面板挪到绘制序列末尾，即抬到所有悬浮面板之上。
    pub(crate) fn raise_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        if !self.manager_mut().raise(editor) {
            return;
        }
        cx.notify();
    }

    /// 标题栏按下：记录抓取偏移并置顶，真正的位移由窗口级监听驱动
    /// （见 `EditorDragTracker`）。
    pub(crate) fn begin_editor_panel_drag(
        &mut self,
        editor: &Entity<EditorState>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.manager_mut().raise(editor);
        let (x, y): (f32, f32) = (position.x.into(), position.y.into());
        // 管理器会顺手清掉其它面板残留的拖拽位，保证同一时刻只有一个面板跟随鼠标。
        self.manager_mut().begin_drag(editor, x, y);
        cx.notify();
    }

    /// 视口级拖拽推进：只有确实有面板在被拖动时才改写状态并重绘。
    pub(crate) fn drag_editor_panels(
        &mut self,
        position: Point<Pixels>,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let (x, y): (f32, f32) = (position.x.into(), position.y.into());
        let (viewport_w, viewport_h): (f32, f32) = (viewport.width.into(), viewport.height.into());
        if self.manager_mut().drag(x, y, viewport_w, viewport_h) {
            cx.notify();
        }
    }

    pub(crate) fn end_editor_panel_drag(&mut self, cx: &mut Context<Self>) {
        if !self.manager_mut().end_drag() {
            return;
        }
        cx.notify();
    }

    /// 关闭请求：脏内容先在面板内确认，干净内容直接回收。
    pub(crate) fn request_close_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.map_panel(editor, |item| item.dirty).unwrap_or(false);
        if dirty {
            self.update_panel(editor, |item| {
                item.confirm = Some(PanelConfirm::DiscardClose);
            });
            cx.notify();
            return;
        }
        self.drop_panel(editor);
        self.focus_active_sftp(window, cx);
        cx.notify();
    }

    /// 确认丢弃：放弃未保存改动并关闭面板；若关闭是被标签关闭闸门拦下的，
    /// 还要继续推进那次挂起的关闭。
    pub(crate) fn discard_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_panel(editor, |item| {
            item.confirm = None;
            item.dirty = false;
        });
        self.drop_panel(editor);
        self.focus_active_sftp(window, cx);
        self.defer_close_resume(window, cx);
        cx.notify();
    }

    /// 留在面板：撤销二次确认并把焦点交还编辑器。
    ///
    /// 用户明确选择继续编辑，连带取消挂起的标签关闭意图：否则其余面板处理完
    /// 后自动续行关闭，会把用户刚说要留下的窗格整个关掉。
    pub(crate) fn keep_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_panel(editor, |item| {
            item.confirm = None;
        });
        self.clear_pending_close();
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// 发起恢复确认：只置确认横幅，真正的恢复在 `confirm_revert_editor_panel`。
    pub(crate) fn ask_revert_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        self.update_panel(editor, |item| {
            item.confirm = Some(PanelConfirm::Revert);
        });
        cx.notify();
    }

    /// 确认放弃修改：把缓冲区整体恢复到最近一次与远端一致的内容。
    ///
    /// `set_value` 在库内会重置撤销栈，恢复结果无法用 Ctrl+Z 找回，
    /// 因此必须经由 `PanelConfirm::Revert` 二次确认后才执行到这里。
    pub(crate) fn confirm_revert_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(baseline) = self.map_panel(editor, |item| {
            item.confirm = None;
            item.baseline.clone()
        }) else {
            return;
        };
        editor.update(cx, |state, cx| state.set_value(baseline, window, cx));
        self.update_panel(editor, |item| {
            item.dirty = false;
            item.phase = EditorPhase::Idle;
            item.written_size = None;
            item.error = None;
            // 差异预览展示的是"待回传变更"，内容已回到基线，预览随之失效。
            item.showing_diff = false;
            item.diff = None;
        });
        self.defer_close_resume(window, cx);
        cx.notify();
    }

    /// 收起冲突横幅但保留冲突事实：用户想先继续编辑，缓冲区内容不动，
    /// 下次保存仍会携带旧指纹比对并再次提醒。
    pub(crate) fn dismiss_editor_conflict(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_panel(editor, |item| {
            item.conflict = false;
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// 面板内保存快捷键：焦点在哪个面板就存哪个。
    pub(crate) fn on_save_editor_panel(
        &mut self,
        _: &crate::keymap::SaveEditorFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.focused_editor(window, cx) else {
            return;
        };
        self.save_editor_panel(&editor, window, cx);
    }

    /// 面板内"跳转到行"快捷键：找到焦点面板后由外壳打开行号输入模态。
    pub(crate) fn on_goto_editor_line(
        &mut self,
        _: &crate::keymap::GoToEditorLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.focused_editor(window, cx) else {
            return;
        };
        self.with_shell(cx, |shell, cx| {
            shell.open_editor_goto_line(editor, window, cx)
        });
    }

    fn drop_panel(&mut self, editor: &Entity<EditorState>) {
        self.manager_mut()
            .panels
            .retain(|item| item.editor != *editor);
    }

    /// 面板移除后把焦点交还当前 SFTP 面板。
    fn focus_active_sftp(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.with_shell(cx, |shell, cx| shell.focus_active_sftp_panel(window, cx));
    }

    /// 挂起关闭可能在任意面板处理完后满足；延后到本次实体租借结束后再续行，
    /// 避免外壳续行时重新读写本实体造成双重租借。
    fn defer_close_resume(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.with_shell(cx, |shell, cx| shell.defer_editor_close_resume(window, cx));
    }
}
