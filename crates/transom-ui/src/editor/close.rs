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
