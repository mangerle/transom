//! 编辑面板与标签关闭的协同：受影响标签下存在未保存编辑时先挂起关闭，
//! 待用户逐面板处理（保存 / 丢弃 / 留下）后再自动续行原关闭意图。
//!
//! 打开与读取在 `editor_open.rs`，保存回传在 `editor_ops.rs`，
//! 面板状态在 `editor_panel.rs`，渲染在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 单面板关闭本就有丢弃确认，而关闭标签曾会绕过确认连带丢掉其下全部脏面板，
//!   保护不对称。闸门把标签级关闭降到与单面板同一保护水位。
//! - 挂起而非阻塞：确认横幅挂在各面板内，全部处理完毕自动续行原意图；用户在
//!   任一面板选择"留下继续编辑"则整笔关闭意图取消（见 `keep_editor_panel`）。
//! - 代价与局限：续行按意图标识重新解析受影响标签，挂起期间用户又把某面板改脏
//!   会让续行继续等待——宁可多等一次确认，也不冒丢改动的险。

use super::panel::PanelConfirm;
use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GroupId, SessionId, TabId};

/// 被脏面板拦下、待确认后续行的关闭意图。
///
/// 变体对应会连带丢面板的关闭路径，保存标识而非标签集合：续行时按当前
/// 结构重新解析受影响标签，避免持有过期集合（挂起期间窗格可能被拆分重组）。
/// 并排组关闭是一个多组意图，因此持有组 ID 列表而不是单组 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingEditorClose {
    /// 关闭整个标签组（含其中全部窗格）。
    Group(GroupId),
    /// 关闭单个窗格；组内仅剩该窗格时等价于关闭整组。
    Pane(TabId),
    /// 删除会话（连带其全部标签）。
    Session(SessionId),
    /// 关闭整个并排组（含组内每个标签组及其窗格）。
    Tiling(Vec<GroupId>),
}

impl AppShell {
    /// 关闭路径的脏面板闸门：受影响标签下存在未保存编辑时挂起关闭、弹出各面板
    /// 的丢弃确认横幅并展开置顶。返回 true 表示已拦截，调用方应立即原样返回。
    pub(crate) fn block_close_for_dirty_editors(
        &mut self,
        tab_ids: &[TabId],
        intent: PendingEditorClose,
        cx: &mut Context<Self>,
    ) -> bool {
        let dirty_editors: Vec<_> = self
            .editors
            .panels
            .iter()
            .filter(|item| tab_ids.contains(&item.tab_id) && item.dirty)
            .map(|item| item.editor.clone())
            .collect();
        if dirty_editors.is_empty() {
            return false;
        }
        self.editors.pending_close = Some(intent);
        for editor in dirty_editors {
            if let Some(item) = self.editor_panel_mut(&editor) {
                // 确认横幅渲染在主体区：折叠会把它整个藏掉，先展开；冲突横幅
                // 与确认横幅同区互斥展示，关闭意图优先，先收起冲突。
                item.confirm = Some(PanelConfirm::DiscardClose);
                item.conflict = false;
                item.collapsed = false;
            }
            self.raise_editor_panel(&editor, cx);
        }
        cx.notify();
        true
    }

    /// 脏面板全部处理完毕后续行挂起的关闭；由 dirty 清零的各条路径（保存成功、
    /// 确认丢弃、确认恢复）调用。仍有脏面板则保留意图等待下次触发。
    pub(crate) fn resume_pending_editor_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(intent) = self.editors.pending_close.clone() else {
            return;
        };
        let tab_ids = self.tabs_for_pending_close(&intent);
        if tab_ids.is_empty() {
            // 目标标签已全部消失：关闭已在别处发生或不再相关，意图作废。
            self.editors.pending_close = None;
            cx.notify();
            return;
        }
        let still_dirty = self
            .editors
            .panels
            .iter()
            .any(|item| tab_ids.contains(&item.tab_id) && item.dirty);
        if still_dirty {
            return;
        }
        self.editors.pending_close = None;
        match intent {
            PendingEditorClose::Group(group_id) => self.close_group(group_id, window, cx),
            PendingEditorClose::Pane(tab_id) => self.close_pane(tab_id, window, cx),
            PendingEditorClose::Session(session_id) => self.delete_session_by_id(session_id, cx),
            PendingEditorClose::Tiling(groups) => {
                self.close_tiling_groups(groups, window, cx);
            }
        }
    }

    /// 按意图标识解析当前受影响的标签集合；空集表示目标已不存在。
    fn tabs_for_pending_close(&self, intent: &PendingEditorClose) -> Vec<TabId> {
        match intent {
            PendingEditorClose::Group(group_id) => self
                .workspace
                .groups
                .iter()
                .find(|group| group.id == *group_id)
                .map(|group| group.pane_ids())
                .unwrap_or_default(),
            PendingEditorClose::Pane(tab_id) => Vec::from([*tab_id]),
            PendingEditorClose::Session(session_id) => self
                .workspace
                .tabs
                .iter()
                .filter(|tab| tab.session_id == *session_id)
                .map(|tab| tab.id)
                .collect(),
            PendingEditorClose::Tiling(groups) => groups
                .iter()
                .filter_map(|group_id| {
                    self.workspace
                        .groups
                        .iter()
                        .find(|group| group.id == *group_id)
                })
                .flat_map(|group| group.pane_ids())
                .collect(),
        }
    }

    /// 丢弃标签已不存在的编辑面板。
    ///
    /// 面板挂在 `TabId`（窗格）上，关闭标签、关闭窗格、关闭整组都会让面板失去宿主；
    /// 集中在一处按存活标签集合裁剪，比在每条关闭路径里分别点名更可靠，订阅句柄随
    /// 面板一起析构退订，不会留下向已死实体投递的事件。
    pub(crate) fn prune_editor_panels(&mut self) {
        if self.editors.is_empty() {
            return;
        }
        let alive: Vec<TabId> = self.workspace.tabs.iter().map(|tab| tab.id).collect();
        self.editors.prune_dead_tabs(&alive);
    }
}
