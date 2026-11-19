//! 标签组及其通用归属键。

use crate::pane_tree::LayoutNode;
use transom_core::{GroupId, SessionId, TabId};

/// 标签组的归属方式。
///
/// 设计原理（Why）：终端分屏按会话归组，而 Git、容器、播放器和后续 API
/// 工作区没有自己的终端会话。用归属枚举替代裸 `SessionId` 后，非会话标签不再
/// 需要伪造会话才能参与标签组和并排网格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TabGroupOwner {
    Session(SessionId),
    Workspace(TabId),
}

/// 标签组：一个标签栏条目下挂载的一棵分割树。
pub(crate) struct TabGroup {
    pub(crate) id: GroupId,
    pub(crate) owner: TabGroupOwner,
    pub(crate) serial: usize,
    pub(crate) root: LayoutNode,
    pub(crate) active_pane: TabId,
    pub(crate) zoomed: Option<TabId>,
}

impl TabGroup {
    pub(crate) fn new(pane: TabId, owner: TabGroupOwner, serial: usize) -> Self {
        Self {
            id: GroupId::new(),
            owner,
            serial,
            root: LayoutNode::pane(pane),
            active_pane: pane,
            zoomed: None,
        }
    }

    pub(crate) fn pane_ids(&self) -> Vec<TabId> {
        self.root.pane_ids()
    }

    pub(crate) fn contains(&self, tab: TabId) -> bool {
        self.root.contains(tab)
    }

    pub(crate) fn pane_count(&self) -> usize {
        self.root.pane_count()
    }
}
