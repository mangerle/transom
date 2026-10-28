//! 远端文本编辑面板状态管理器。
//!
//! 设计原理（Why）：
//! 将编辑面板集合、待关闭拦截意图与面板列表弹窗状态从顶层 `AppShell` 抽离，
//! 实现子系统的自治状态管理与职责内聚，消除上帝对象的平铺字段堆砌。

use super::close::PendingEditorClose;
use super::panel::EditorPanelState;
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

    pub(crate) fn is_empty(&self) -> bool {
        self.panels.is_empty()
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
