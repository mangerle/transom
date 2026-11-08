//! 编辑面板与标签关闭的协同：受影响标签下存在未保存编辑时先挂起关闭，
//! 待用户逐面板处理（保存 / 丢弃 / 留下）后再自动续行原关闭意图。
//!
//! 打开与读取在 `open.rs`，保存回传在 `ops.rs`，面板状态在 `panel.rs`，
//! 渲染在 `view.rs`。
//!
//! 设计原理（Why）：
//! - 单面板关闭本就有丢弃确认，而关闭标签曾会绕过确认连带丢掉其下全部脏面板，
//!   保护不对称。闸门把标签级关闭降到与单面板同一保护水位。
//! - 挂起而非阻塞：确认横幅挂在各面板内，全部处理完毕自动续行原意图；用户在
//!   任一面板选择"留下继续编辑"则整笔关闭意图取消（见 `keep_editor_panel`）。
//! - 代价与局限：续行按意图标识重新解析受影响标签，挂起期间用户又把某面板改脏
//!   会让续行继续等待——宁可多等一次确认，也不冒丢改动的险。
//!
//! 实体化边界：挂起意图与脏面板判定都归 [`EditorPanels`]；外壳只负责按意图
//! 标识解析当前工作区标签并执行真正的关闭动作。

use super::entity::EditorPanels;
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

impl EditorPanels {
    /// 关闭路径的脏面板闸门：受影响标签下存在未保存编辑时挂起关闭、弹出各面板
    /// 的丢弃确认横幅并展开置顶。返回 true 表示已拦截，调用方应立即原样返回。
    pub(crate) fn block_close(
        &mut self,
        tab_ids: &[TabId],
        intent: PendingEditorClose,
        cx: &mut Context<Self>,
    ) -> bool {
        let dirty_editors: Vec<_> = self
            .manager()
            .panels
            .iter()
            .filter(|item| tab_ids.contains(&item.tab_id) && item.dirty)
            .map(|item| item.editor.clone())
            .collect();
        if dirty_editors.is_empty() {
            return false;
        }
        self.manager_mut().pending_close = Some(intent);
        for editor in dirty_editors {
            self.update_panel(&editor, |item| {
                // 确认横幅渲染在主体区：折叠会把它整个藏掉，先展开；冲突横幅
                // 与确认横幅同区互斥展示，关闭意图优先，先收起冲突。
                item.confirm = Some(PanelConfirm::DiscardClose);
                item.conflict = false;
                item.collapsed = false;
            });
            self.manager_mut().raise(&editor);
        }
        cx.notify();
        true
    }
}

impl AppShell {
    /// 丢弃标签已不存在的对比面板。
    ///
    /// 对比面板挂在 `TabId`（窗格）上，关闭标签、关闭窗格、关闭整组都会让它
    /// 失去宿主；编辑面板的同类裁剪由 [`AppShell::prune_tab_overlays`] 转交实体。
    pub(crate) fn prune_tab_diff_overlay(&mut self, cx: &App) {
        let Some(view) = self.state.diff_viewer.as_ref() else {
            return;
        };
        let tab_id = view.read(cx).state.tab_id;
        let alive = self.state.workspace.tabs.iter().any(|tab| tab.id == tab_id);
        if !alive {
            self.state.diff_viewer = None;
            self.state.tasks.subscriptions.diff_viewer = None;
        }
    }
}

impl AppShell {
    /// 按存活标签集合裁剪编辑面板、对比面板与容器工作区订阅。
    ///
    /// 编辑面板与对比面板都挂在 `TabId`（窗格）上，关闭标签、关闭窗格、关闭整组
    /// 都会让面板失去宿主；容器工作区的事件订阅同样按标签持有，实体销毁后继续
    /// 留在映射里就是泄漏。集中在一处裁剪比在每条关闭路径里分别点名更可靠，
    /// 订阅句柄随面板一起析构退订，不会留下向已死实体投递的事件。
    pub(crate) fn prune_tab_overlays(&mut self, cx: &mut Context<Self>) {
        let alive: Vec<TabId> = self.state.workspace.tabs.iter().map(|tab| tab.id).collect();
        self.state
            .editors
            .update(cx, |panels, _| panels.prune_dead_tabs(&alive));
        self.state
            .tasks
            .subscriptions
            .prune_container_workspaces(&alive);
        self.prune_tab_diff_overlay(cx);
    }

    /// 关闭路径的脏面板闸门；实体决定是否拦截并展开确认横幅。
    pub(crate) fn block_close_for_dirty_editors(
        &mut self,
        tab_ids: &[TabId],
        intent: PendingEditorClose,
        cx: &mut Context<Self>,
    ) -> bool {
        self.state
            .editors
            .update(cx, |panels, cx| panels.block_close(tab_ids, intent, cx))
    }

    /// 脏面板全部处理完毕后续行挂起的关闭；由 dirty 清零的各条路径（保存成功、
    /// 确认丢弃、确认恢复）调用。仍有脏面板则保留意图等待下次触发。
    pub(crate) fn resume_pending_editor_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(intent) = self.state.editors.read(cx).pending_close() else {
            return;
        };
        let dirty_tabs = self.state.editors.read(cx).dirty_tabs();
        self.try_finish_pending_close(intent, &dirty_tabs, window, cx);
    }

    /// 在一次实体租借结束后再续行关闭：实体回调里直接续行会重新读写本实体，
    /// 触发双重租借崩溃，因此统一延迟到下一个空转点。
    pub(crate) fn defer_editor_close_resume(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.defer_in(window, |shell, window, cx| {
            shell.resume_pending_editor_close(window, cx)
        });
    }

    /// 按挂起意图与当前脏标签快照决定是否续行；由保存成功等外壳事件复用。
    pub(super) fn try_finish_pending_close(
        &mut self,
        intent: PendingEditorClose,
        dirty_tabs: &[TabId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_ids = self.tabs_for_pending_close(&intent);
        if tab_ids.is_empty() {
            // 目标标签已全部消失：关闭已在别处发生或不再相关，意图作废。
            self.clear_editor_pending_close(cx);
            cx.notify();
            return;
        }
        if tab_ids.iter().any(|tab_id| dirty_tabs.contains(tab_id)) {
            return;
        }
        self.clear_editor_pending_close(cx);
        match intent {
            PendingEditorClose::Group(group_id) => self.close_group(group_id, window, cx),
            PendingEditorClose::Pane(tab_id) => self.close_pane(tab_id, window, cx),
            PendingEditorClose::Session(session_id) => self.delete_session_by_id(session_id, cx),
            PendingEditorClose::Tiling(groups) => {
                self.close_tiling_groups(groups, window, cx);
            }
        }
    }

    fn clear_editor_pending_close(&mut self, cx: &mut Context<Self>) {
        self.state
            .editors
            .update(cx, |panels, _| panels.clear_pending_close());
    }

    /// 按意图标识解析当前受影响的标签集合；空集表示目标已不存在。
    fn tabs_for_pending_close(&self, intent: &PendingEditorClose) -> Vec<TabId> {
        match intent {
            PendingEditorClose::Group(group_id) => self
                .state
                .workspace
                .groups
                .iter()
                .find(|group| group.id == *group_id)
                .map(|group| group.pane_ids())
                .unwrap_or_default(),
            PendingEditorClose::Pane(tab_id) => Vec::from([*tab_id]),
            PendingEditorClose::Session(session_id) => self
                .state
                .workspace
                .tabs
                .iter()
                .filter(|tab| tab.session_id == *session_id)
                .map(|tab| tab.id)
                .collect(),
            PendingEditorClose::Tiling(groups) => groups
                .iter()
                .filter_map(|group_id| {
                    self.state
                        .workspace
                        .groups
                        .iter()
                        .find(|group| group.id == *group_id)
                })
                .flat_map(|group| group.pane_ids())
                .collect(),
        }
    }
}
