//! 标签组分屏二叉分割树的核心数据模型与方向定义。

use transom_core::{SavedLayout, SplitAxis, SplitId, TabId};

/// 单个标签组允许的窗格上限。
///
/// 每个窗格都是一条独立连接（PTY、心跳、统计订阅各一份），上限既要挡住连续
/// 误按快捷键造成的连接风暴，也要保证窄窗口下每个窗格仍有可读宽度。它同时把
/// 运行期树的深度锁在 `MAX_PANES - 1` 层以内。
pub(crate) const MAX_PANES: usize = 6;

/// 焦点在窗格之间移动的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

impl PaneDirection {
    /// 该方向落在哪个切分轴上：左右由水平切分产生，上下由垂直切分产生。
    pub(crate) fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Horizontal,
            Self::Up | Self::Down => SplitAxis::Vertical,
        }
    }

    /// 朝该方向前进时，是否应进入祖先节点的兄弟子树。
    pub(crate) fn enters_sibling(self, came_from_second: bool) -> bool {
        match self {
            Self::Right | Self::Down => !came_from_second,
            Self::Left | Self::Up => came_from_second,
        }
    }

    /// 落到兄弟子树后取哪个窗格：顺方向过去取最靠前的，逆方向回来取最靠后的。
    pub(crate) fn picks_first(self) -> bool {
        match self {
            Self::Right | Self::Down => true,
            Self::Left | Self::Up => false,
        }
    }
}

/// 摘除叶子的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoveOutcome {
    /// 树里已没有窗格：调用方应销毁整个标签组。
    Empty,
    /// 已收起父节点，`next_focus` 为原窗格在同轴上的相邻窗格。
    Removed { next_focus: TabId },
}

/// 分割树节点。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LayoutNode {
    /// 叶子窗格，值为 `TerminalTab` 的标识。
    Pane(TabId),
    /// 内部节点：沿 `axis` 把可用空间分给两个子节点。
    Split {
        /// 节点标识，渲染时用作可拖拽容器的 ElementId。必须稳定。
        id: SplitId,
        axis: SplitAxis,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    /// 以单个窗格作为一棵树，即新建标签组的初始布局。
    pub(crate) fn pane(tab: TabId) -> Self {
        Self::Pane(tab)
    }

    pub(crate) fn split(axis: SplitAxis, first: Self, second: Self) -> Self {
        Self::Split {
            id: SplitId::new(),
            axis,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// 根节点的切分方向，供标签栏徽标显示；单窗格返回默认方向。
    pub(crate) fn root_axis(&self) -> SplitAxis {
        match self {
            Self::Pane(_) => SplitAxis::default(),
            Self::Split { axis, .. } => *axis,
        }
    }

    /// 窗格总数（叶子数）。
    pub(crate) fn pane_count(&self) -> usize {
        let mut count = 0;
        let mut stack = Vec::with_capacity(MAX_PANES);
        stack.push(self);
        while let Some(node) = stack.pop() {
            match node {
                Self::Pane(_) => count += 1,
                Self::Split { first, second, .. } => {
                    stack.push(first);
                    stack.push(second);
                }
            }
        }
        count
    }

    pub(crate) fn contains(&self, tab: TabId) -> bool {
        self.pane_ids().contains(&tab)
    }

    /// 按视觉顺序（从左到右、从上到下）列出全部窗格。
    pub(crate) fn pane_ids(&self) -> Vec<TabId> {
        let mut out = Vec::with_capacity(MAX_PANES);
        let mut stack = Vec::with_capacity(MAX_PANES);
        stack.push(self);
        while let Some(node) = stack.pop() {
            match node {
                Self::Pane(tab) => out.push(*tab),
                Self::Split { first, second, .. } => {
                    stack.push(second);
                    stack.push(first);
                }
            }
        }
        out
    }

    /// 全部分割节点标识，供渲染侧清理不再存在的拖拽状态。
    pub(crate) fn split_ids(&self) -> Vec<SplitId> {
        let mut out = Vec::with_capacity(MAX_PANES);
        let mut stack = Vec::with_capacity(MAX_PANES);
        stack.push(self);
        while let Some(node) = stack.pop() {
            if let Self::Split {
                id, first, second, ..
            } = node
            {
                out.push(*id);
                stack.push(first);
                stack.push(second);
            }
        }
        out
    }

    /// 最靠前（左上）的窗格。
    pub(crate) fn first_pane(&self) -> Option<TabId> {
        let mut node = self;
        while let Self::Split { first, .. } = node {
            node = first;
        }
        match node {
            Self::Pane(tab) => Some(*tab),
            Self::Split { .. } => None,
        }
    }

    /// 最靠后（右下）的窗格。
    pub(crate) fn last_pane(&self) -> Option<TabId> {
        let mut node = self;
        while let Self::Split { second, .. } = node {
            node = second;
        }
        match node {
            Self::Pane(tab) => Some(*tab),
            Self::Split { .. } => None,
        }
    }

    /// 导出为持久化形状。运行期树深度受 `MAX_PANES` 约束，递归栈深度确定。
    pub(crate) fn to_saved(&self) -> SavedLayout {
        match self {
            Self::Pane(_) => SavedLayout::Pane,
            Self::Split {
                axis,
                first,
                second,
                ..
            } => SavedLayout::Split {
                axis: *axis,
                first: Box::new(first.to_saved()),
                second: Box::new(second.to_saved()),
            },
        }
    }
}

/// 判断子节点是否恰为 `target` 叶子：分割树里叶子的父节点必直接持有它。
pub(crate) fn is_leaf(node: &LayoutNode, target: TabId) -> bool {
    matches!(node, LayoutNode::Pane(tab) if *tab == target)
}
