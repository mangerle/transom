//! SFTP 编辑面板的窗体动作：折叠/最大化、置顶与拖拽、关闭与恢复的确认流程。
//!
//! 面板状态与几何换算在 `editor_panel.rs`，打开与读取在 `editor_open.rs`，
//! 保存回传在 `editor_ops.rs`，关闭闸门在 `editor_close.rs`，渲染在 `editor_view.rs`。

use super::editor_panel::{EditorPhase, PANEL_KEEP_VISIBLE, PanelConfirm, clamp_axis};
use crate::app::AppShell;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn toggle_editor_panel_collapsed(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        item.collapsed = !item.collapsed;
        // 折叠与最大化互斥：折叠时先退出全屏，否则只有标题栏的一条横幅铺满整屏。
        if item.collapsed {
            item.maximized = false;
        }
        cx.notify();
    }

    pub(crate) fn toggle_editor_panel_maximized(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        item.maximized = !item.maximized;
        item.collapsed = false;
        cx.notify();
    }

    /// 把面板挪到绘制序列末尾，即抬到所有悬浮面板之上。
    pub(crate) fn raise_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editor_panels
            .iter()
            .position(|item| item.editor == *editor)
        else {
            return;
        };
        if index + 1 == self.editor_panels.len() {
            return;
        }
        let item = self.editor_panels.remove(index);
        self.editor_panels.push(item);
        cx.notify();
    }

    /// 标题栏按下：记录抓取偏移并置顶，真正的位移由视口级 mouse_move 驱动。
    pub(crate) fn begin_editor_panel_drag(
        &mut self,
        editor: &Entity<EditorState>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.raise_editor_panel(editor, cx);
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        if item.maximized {
            return;
        }
        let (x, y): (f32, f32) = (position.x.into(), position.y.into());
        item.drag_grab = Some((x - item.left, y - item.top));
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
        let mut dragging = false;
        for item in self.editor_panels.iter_mut() {
            let Some((grab_x, grab_y)) = item.drag_grab else {
                continue;
            };
            dragging = true;
            item.left = clamp_axis(x - grab_x, item.width, viewport_w, PANEL_KEEP_VISIBLE);
            item.top = clamp_axis(
                y - grab_y,
                item.current_height(),
                viewport_h,
                PANEL_KEEP_VISIBLE,
            );
        }
        if dragging {
            cx.notify();
        }
    }

    pub(crate) fn end_editor_panel_drag(&mut self, cx: &mut Context<Self>) {
        if !self
            .editor_panels
            .iter()
            .any(|item| item.drag_grab.is_some())
        {
            return;
        }
        for item in self.editor_panels.iter_mut() {
            item.drag_grab = None;
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
        let dirty = self
            .editor_panels
            .iter()
            .find(|item| item.editor == *editor)
            .is_some_and(|item| item.dirty);
        if dirty {
            if let Some(item) = self.editor_panel_mut(editor) {
                item.confirm = Some(PanelConfirm::DiscardClose);
            }
            cx.notify();
            return;
        }
        self.drop_editor_panel(editor, window, cx);
    }

    /// 确认丢弃：放弃未保存改动并关闭面板；若关闭是被标签关闭闸门拦下的，
    /// 还要继续推进那次挂起的关闭。
    pub(crate) fn discard_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.editor_panel_mut(editor) {
            item.confirm = None;
            item.dirty = false;
        }
        self.drop_editor_panel(editor, window, cx);
        self.resume_pending_editor_close(window, cx);
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
        if let Some(item) = self.editor_panel_mut(editor) {
            item.confirm = None;
        }
        self.pending_editor_close = None;
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// 发起恢复确认：只置确认横幅，真正的恢复在 `confirm_revert_editor_panel`。
    pub(crate) fn ask_revert_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.editor_panel_mut(editor) {
            item.confirm = Some(PanelConfirm::Revert);
        }
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
        let baseline = if let Some(item) = self.editor_panel_mut(editor) {
            item.confirm = None;
            item.baseline.clone()
        } else {
            return;
        };
        editor.update(cx, |state, cx| state.set_value(baseline, window, cx));
        if let Some(item) = self.editor_panel_mut(editor) {
            item.dirty = false;
            item.phase = EditorPhase::Idle;
            item.written_size = None;
            item.error = None;
            // 差异预览展示的是"待回传变更"，内容已回到基线，预览随之失效。
            item.showing_diff = false;
            item.diff = None;
        }
        self.resume_pending_editor_close(window, cx);
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
        if let Some(item) = self.editor_panel_mut(editor) {
            item.conflict = false;
        }
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
}
