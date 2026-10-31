//! 远端文本编辑面板状态管理器。
//!
//! 设计原理（Why）：
//! 将编辑面板集合、待关闭拦截意图与面板列表弹窗状态从顶层 `AppShell` 抽离，
//! 实现子系统的自治状态管理与职责内聚，消除上帝对象的平铺字段堆砌。

use super::close::PendingEditorClose;
use super::panel::EditorPanelState;
use super::panel::{PANEL_KEEP_VISIBLE, clamp_axis};
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::TabId;

/// 远端文本编辑面板的自包含管理器。
#[derive(Default)]
pub(crate) struct EditorManager {
    /// 已打开的远端文本编辑面板集合。
    pub(crate) panels: Vec<EditorPanelState>,
    /// 被脏编辑面板拦下的标签/会话关闭意图。
    pub(crate) pending_close: Option<PendingEditorClose>,
    /// 工具栏"已打开编辑器"浮层的受控显隐状态。
    pub(crate) list_open: bool,
}

impl EditorManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.panels.len()
    }

    /// 清理已关闭标签所对应的悬浮编辑器面板。
    pub(crate) fn prune_dead_tabs(&mut self, active_tabs: &[TabId]) {
        if self.panels.is_empty() {
            return;
        }
        self.panels
            .retain(|item| active_tabs.contains(&item.tab_id));
    }
}
impl EditorManager {
    /// 找面板下标。
    fn index_of(&self, editor: &Entity<EditorState>) -> Option<usize> {
        self.panels.iter().position(|item| item.editor == *editor)
    }

    /// 折叠/展开；折叠与最大化互斥，折叠时退出全屏。返回是否找到面板。
    pub(crate) fn toggle_collapsed(&mut self, editor: &Entity<EditorState>) -> bool {
        let Some(index) = self.index_of(editor) else {
            return false;
        };
        let item = &mut self.panels[index];
        item.collapsed = !item.collapsed;
        if item.collapsed {
            item.maximized = false;
        }
        true
    }

    /// 最大化/还原。返回是否找到面板。
    pub(crate) fn toggle_maximized(&mut self, editor: &Entity<EditorState>) -> bool {
        let Some(index) = self.index_of(editor) else {
            return false;
        };
        let item = &mut self.panels[index];
        item.maximized = !item.maximized;
        item.collapsed = false;
        true
    }

    /// 把面板挪到绘制序列末尾，即抬到所有悬浮面板之上。返回是否发生重排。
    pub(crate) fn raise(&mut self, editor: &Entity<EditorState>) -> bool {
        let Some(index) = self.index_of(editor) else {
            return false;
        };
        if index + 1 == self.panels.len() {
            return false;
        }
        let item = self.panels.remove(index);
        self.panels.push(item);
        true
    }

    /// 标题栏按下：记录抓取偏移（仅一个面板可处于拖拽中）。
    pub(crate) fn begin_drag(&mut self, editor: &Entity<EditorState>, x: f32, y: f32) {
        for other in self.panels.iter_mut() {
            if other.editor != *editor {
                other.drag_grab = None;
            }
        }
        let Some(index) = self.index_of(editor) else {
            return;
        };
        let item = &mut self.panels[index];
        if item.maximized {
            return;
        }
        item.drag_grab = Some((x - item.left, y - item.top));
    }

    /// 视口级拖拽推进；返回是否有面板被移动。
    pub(crate) fn drag(&mut self, x: f32, y: f32, viewport_w: f32, viewport_h: f32) -> bool {
        let mut dragging = false;
        for item in self.panels.iter_mut() {
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
        dragging
    }

    /// 结束拖拽；返回此前是否有面板在拖。
    pub(crate) fn end_drag(&mut self) -> bool {
        if !self.panels.iter().any(|item| item.drag_grab.is_some()) {
            return false;
        }
        for item in self.panels.iter_mut() {
            item.drag_grab = None;
        }
        true
    }

    /// 是否有面板处于拖拽中。
    pub(crate) fn drag_in_progress(&self) -> bool {
        self.panels.iter().any(|item| item.drag_grab.is_some())
    }
}
