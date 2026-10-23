//! 标签组内的分割树：窗格布局的唯一事实来源。
//!
//! 采用二叉分割树而不是扁平窗格列表，是为了让"切分方向"成为每个分割节点
//! 自身的属性：在左右并排的窗格上按"向下分屏"只把焦点窗格切一半，兄弟窗格
//! 保持原尺寸 —— 这与 iTerm2 / Windows Terminal 的空间直觉一致。扁平列表只能
//! 整体重排（方向是标签组的属性），那是旧实现交互违和的根源。
//!
//! 代价是布局不再是"一个数组 + 一个方向"：摘除窗格要收起退化的父节点，
//! 移动焦点要沿祖先链判断方向。这些复杂度全部收敛在本文件，上层
//! （分屏动作、渲染、持久化）只使用"切分 / 摘除 / 翻转 / 找邻居"四个动词。

use transom_core::{SavedLayout, SplitAxis, SplitId, TabId};

/// 单个标签组允许的窗格上限。
///
/// 每个窗格都是一条独立连接（PTY、心跳、统计订阅各一份），上限既要挡住连续
/// 误按快捷键造成的连接风暴，也要保证窄窗口下每个窗格仍有可读宽度。它同时把
/// 运行期树的深度锁在 `MAX_PANES - 1` 层以内，本文件对运行期树的递归遍历因此
/// 都有确定的栈深度上界；来自设置文件的形状另走显式栈构建，见 `from_saved`。
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
    fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Horizontal,
            Self::Up | Self::Down => SplitAxis::Vertical,
        }
    }

    /// 朝该方向前进时，是否应进入祖先节点的兄弟子树。
    ///
    /// `came_from_second` 表示下潜时走的是该轴的后侧（右侧/下侧）子节点：此时
    /// 向左/向上才是"回到兄弟"，向右/向下说明这一层已到头，需要继续向外层询问。
    fn enters_sibling(self, came_from_second: bool) -> bool {
        match self {
            Self::Right | Self::Down => !came_from_second,
            Self::Left | Self::Up => came_from_second,
        }
    }

    /// 落到兄弟子树后取哪个窗格：顺方向过去取最靠前的，逆方向回来取最靠后的。
    fn picks_first(self) -> bool {
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
        /// 节点标识，渲染时用作可拖拽容器的 ElementId。必须稳定：子树收起或
        /// 兄弟被摘除后，重建祖先节点要沿用原标识，用户拖出的比例才不丢。
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

    fn split(axis: SplitAxis, first: Self, second: Self) -> Self {
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
                // 先压后侧再压前侧，弹出顺序即为视觉顺序。
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

    /// 把 `target` 窗格沿 `axis` 切成两半，新窗格落在其同轴后侧（右或下）。
    ///
    /// 只切焦点窗格自身，兄弟子树原样保留，因此已拖出的比例不受影响。
    /// 达到 `MAX_PANES` 或 `target` 不在树中时返回 `false`，布局保持不变。
    pub(crate) fn split_leaf(&mut self, target: TabId, axis: SplitAxis, new_pane: TabId) -> bool {
        if self.pane_count() >= MAX_PANES {
            return false;
        }
        let tree = std::mem::replace(self, Self::Pane(new_pane));
        let (rebuilt, done) = Self::split_owned(tree, target, axis, new_pane);
        *self = rebuilt;
        done
    }

    fn split_owned(node: Self, target: TabId, axis: SplitAxis, new_pane: TabId) -> (Self, bool) {
        match node {
            Self::Pane(tab) if tab == target => (
                Self::split(axis, Self::Pane(tab), Self::Pane(new_pane)),
                true,
            ),
            leaf @ Self::Pane(_) => (leaf, false),
            Self::Split {
                id,
                axis: own_axis,
                first,
                second,
            } => {
                let (new_first, done) = if first.contains(target) {
                    let (rebuilt, done) = Self::split_owned(*first, target, axis, new_pane);
                    (Box::new(rebuilt), done)
                } else {
                    (first, false)
                };
                if done {
                    return (
                        Self::Split {
                            id,
                            axis: own_axis,
                            first: new_first,
                            second,
                        },
                        true,
                    );
                }
                let (rebuilt, done) = Self::split_owned(*second, target, axis, new_pane);
                (
                    Self::Split {
                        id,
                        axis: own_axis,
                        first: new_first,
                        second: Box::new(rebuilt),
                    },
                    done,
                )
            }
        }
    }

    /// 摘除窗格并收起退化的父节点，焦点落到同轴相邻窗格。
    ///
    /// 摘除前侧（左/上）取兄弟子树最靠前的窗格，摘除后侧（右/下）取兄弟子树
    /// 最靠后的窗格，视线位移最小。
    pub(crate) fn remove_leaf(&mut self, target: TabId) -> RemoveOutcome {
        if matches!(self, Self::Pane(_)) {
            // 根节点即唯一窗格，组内再无其他候选；self 保留该叶子，调用方销毁整组。
            return RemoveOutcome::Empty;
        }
        let tree = std::mem::replace(self, Self::Pane(target));
        let (rebuilt, focus) = Self::remove_owned(tree, target);
        *self = rebuilt;
        match focus {
            Some(next_focus) => RemoveOutcome::Removed { next_focus },
            None => RemoveOutcome::Empty,
        }
    }

    fn remove_owned(node: Self, target: TabId) -> (Self, Option<TabId>) {
        match node {
            leaf @ Self::Pane(_) => (leaf, None),
            Self::Split {
                id,
                axis,
                first,
                second,
            } => {
                if is_leaf(&first, target) {
                    let surviving = *second;
                    let focus = surviving.first_pane();
                    return (surviving, focus);
                }
                if is_leaf(&second, target) {
                    let surviving = *first;
                    let focus = surviving.last_pane();
                    return (surviving, focus);
                }
                if first.contains(target) {
                    let (rebuilt, focus) = Self::remove_owned(*first, target);
                    (
                        Self::Split {
                            id,
                            axis,
                            first: Box::new(rebuilt),
                            second,
                        },
                        focus,
                    )
                } else {
                    let (rebuilt, focus) = Self::remove_owned(*second, target);
                    (
                        Self::Split {
                            id,
                            axis,
                            first,
                            second: Box::new(rebuilt),
                        },
                        focus,
                    )
                }
            }
        }
    }

    /// 翻转 `target` 所在那一刀（其父节点）的方向，外层结构与兄弟尺寸不动。
    ///
    /// 语义对齐 Windows Terminal 的"切换分屏方向"：目标是焦点窗格与其兄弟之间
    /// 的分割线，而不是整组重排。`target` 不在树中时返回 `false`。
    pub(crate) fn toggle_axis(&mut self, target: TabId) -> bool {
        let tree = std::mem::replace(self, Self::Pane(target));
        let (rebuilt, done) = Self::toggle_owned(tree, target);
        *self = rebuilt;
        done
    }

    fn toggle_owned(node: Self, target: TabId) -> (Self, bool) {
        match node {
            leaf @ Self::Pane(_) => (leaf, false),
            Self::Split {
                id,
                axis,
                first,
                second,
            } => {
                if is_leaf(&first, target) || is_leaf(&second, target) {
                    let flipped = match axis {
                        SplitAxis::Horizontal => SplitAxis::Vertical,
                        SplitAxis::Vertical => SplitAxis::Horizontal,
                    };
                    return (
                        Self::Split {
                            id,
                            axis: flipped,
                            first,
                            second,
                        },
                        true,
                    );
                }
                if first.contains(target) {
                    let (rebuilt, done) = Self::toggle_owned(*first, target);
                    (
                        Self::Split {
                            id,
                            axis,
                            first: Box::new(rebuilt),
                            second,
                        },
                        done,
                    )
                } else {
                    let (rebuilt, done) = Self::toggle_owned(*second, target);
                    (
                        Self::Split {
                            id,
                            axis,
                            first,
                            second: Box::new(rebuilt),
                        },
                        done,
                    )
                }
            }
        }
    }

    /// 沿指定方向寻找下一个焦点窗格；该方向已到边时返回 `None`。
    ///
    /// 由内向外逐层检查祖先：该层切分轴与方向一致且兄弟子树在前方即命中，
    /// 否则这一层已到边，继续向外层询问。因此嵌套布局里的方向键语义与
    /// 屏幕上"往那边走一格"一致，而不是简单地取列表下一个。
    pub(crate) fn focus_neighbor(&self, from: TabId, direction: PaneDirection) -> Option<TabId> {
        if self.pane_count() < 2 || !self.contains(from) {
            return None;
        }
        // 自内而外记录祖先：切分轴、下潜时走的是不是后侧子节点、另一侧子树。
        let mut ancestors: Vec<(SplitAxis, bool, &Self)> = Vec::with_capacity(MAX_PANES);
        let mut cursor = self;
        while let Self::Split {
            axis,
            first,
            second,
            ..
        } = cursor
        {
            if first.contains(from) {
                ancestors.push((*axis, false, second));
                cursor = first;
            } else {
                ancestors.push((*axis, true, first));
                cursor = second;
            }
        }
        ancestors
            .iter()
            .rev()
            .find_map(|(axis, came_from_second, sibling)| {
                if *axis != direction.axis() || !direction.enters_sibling(*came_from_second) {
                    return None;
                }
                if direction.picks_first() {
                    sibling.first_pane()
                } else {
                    sibling.last_pane()
                }
            })
    }

    /// 由持久化形状重建运行期树，叶子按视觉顺序消费 `ids`。
    ///
    /// 设置文件属外部输入，嵌套深度与叶子数量都不可信，因此用显式栈构建，
    /// 并在数量超过 `MAX_PANES` 时整棵拒绝，避免损坏数据一次拉起大量连接。
    pub(crate) fn from_saved(
        shape: &SavedLayout,
        ids: &mut impl Iterator<Item = TabId>,
    ) -> Option<Self> {
        Self::saved_pane_count(shape)?;
        enum Task<'a> {
            Build(&'a SavedLayout),
            Join { axis: SplitAxis },
        }
        let mut tasks: Vec<Task> = Vec::with_capacity(MAX_PANES * 2);
        let mut built: Vec<Self> = Vec::with_capacity(MAX_PANES);
        tasks.push(Task::Build(shape));
        while let Some(task) = tasks.pop() {
            match task {
                Task::Build(SavedLayout::Pane) => built.push(Self::Pane(ids.next()?)),
                Task::Build(SavedLayout::Split {
                    axis,
                    first,
                    second,
                }) => {
                    tasks.push(Task::Join { axis: *axis });
                    tasks.push(Task::Build(second));
                    tasks.push(Task::Build(first));
                }
                Task::Join { axis } => {
                    // 前侧先入栈先出结果，故弹 built 时后侧在前。
                    let second = built.pop()?;
                    let first = built.pop()?;
                    built.push(Self::split(axis, first, second));
                }
            }
        }
        if built.len() == 1 { built.pop() } else { None }
    }

    /// 形状的叶子数量；为空或超过上限返回 `None`。
    pub(crate) fn saved_pane_count(shape: &SavedLayout) -> Option<usize> {
        let mut count = 0;
        let mut stack = vec![shape];
        while let Some(node) = stack.pop() {
            match node {
                SavedLayout::Pane => {
                    count += 1;
                    if count > MAX_PANES {
                        return None;
                    }
                }
                SavedLayout::Split { first, second, .. } => {
                    stack.push(second);
                    stack.push(first);
                }
            }
        }
        (count >= 1).then_some(count)
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
fn is_leaf(node: &LayoutNode, target: TabId) -> bool {
    matches!(node, LayoutNode::Pane(tab) if *tab == target)
}

#[cfg(test)]
mod tests {
    use super::{LayoutNode, MAX_PANES, PaneDirection, RemoveOutcome};
    use transom_core::{SavedLayout, SplitAxis, TabId};

    fn tab() -> TabId {
        TabId::new()
    }

    /// 生成 `count` 个窗格、沿同一方向链式切分的形状：`X(X(...(p, p), p), p)`。
    fn chain_saved(count: usize, axis: SplitAxis) -> SavedLayout {
        let mut shape = SavedLayout::Pane;
        for _ in 1..count {
            shape = SavedLayout::Split {
                axis,
                first: Box::new(shape),
                second: Box::new(SavedLayout::Pane),
            };
        }
        shape
    }

    /// 生成 `count` 个窗格、沿同一方向链式切分的树。
    fn chain(count: usize, axis: SplitAxis) -> (LayoutNode, Vec<TabId>) {
        let ids: Vec<TabId> = (0..count).map(|_| tab()).collect();
        let shape = chain_saved(count, axis);
        let mut cursor = ids.iter().copied();
        (LayoutNode::from_saved(&shape, &mut cursor).unwrap(), ids)
    }

    /// 构造 `H(a, V(b, c))`：a 在左，b 在右上，c 在右下。
    fn nested() -> (LayoutNode, [TabId; 3]) {
        let [a, b, c] = [tab(), tab(), tab()];
        let mut root = LayoutNode::pane(a);
        assert!(root.split_leaf(a, SplitAxis::Horizontal, b));
        assert!(root.split_leaf(b, SplitAxis::Vertical, c));
        (root, [a, b, c])
    }

    #[test]
    fn single_pane_split_places_new_pane_after_the_focused_one() {
        let (focused, fresh) = (tab(), tab());
        let mut root = LayoutNode::pane(focused);
        assert!(root.split_leaf(focused, SplitAxis::Vertical, fresh));
        assert_eq!(root.pane_ids(), vec![focused, fresh]);
        assert_eq!(root.root_axis(), SplitAxis::Vertical);
        assert_eq!(root.last_pane(), Some(fresh));
    }

    #[test]
    fn split_nests_instead_of_rebalancing_siblings() {
        let (left, right, fresh) = (tab(), tab(), tab());
        let mut root = LayoutNode::pane(left);
        root.split_leaf(left, SplitAxis::Horizontal, right);
        // 焦点仍在左窗格：新窗格应紧贴左窗格，右窗格依旧是根节点的后侧子树。
        assert!(root.split_leaf(left, SplitAxis::Horizontal, fresh));
        assert_eq!(root.pane_ids(), vec![left, fresh, right]);
        match &root {
            LayoutNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                assert_eq!(*axis, SplitAxis::Horizontal);
                assert_eq!(second.pane_ids(), vec![right]);
                assert_eq!(first.root_axis(), SplitAxis::Horizontal);
            }
            _ => panic!("根节点应保持为分割节点"),
        }
    }

    #[test]
    fn split_rejects_unknown_pane_and_overflow() {
        let (mut root, ids) = chain(3, SplitAxis::Horizontal);
        assert!(!root.split_leaf(tab(), SplitAxis::Vertical, tab()));
        assert_eq!(root.pane_ids(), ids);
        let (mut full, _) = chain(MAX_PANES, SplitAxis::Horizontal);
        let focus = full.first_pane().unwrap();
        assert!(!full.split_leaf(focus, SplitAxis::Vertical, tab()));
        assert_eq!(full.pane_count(), MAX_PANES);
    }

    #[test]
    fn remove_collapses_parent_and_focuses_adjacent_pane() {
        let (a, b, c) = (tab(), tab(), tab());
        let mut root = LayoutNode::pane(a);
        root.split_leaf(a, SplitAxis::Horizontal, b);
        root.split_leaf(b, SplitAxis::Horizontal, c);
        // 结构为 H(a, H(b, c))：摘除前侧 a，焦点落到兄弟子树最靠前的 b。
        assert_eq!(
            root.remove_leaf(a),
            RemoveOutcome::Removed { next_focus: b }
        );
        assert_eq!(root.root_axis(), SplitAxis::Horizontal);
        // 摘除后侧 c，焦点回到前侧子树最靠后的 b。
        assert_eq!(
            root.remove_leaf(c),
            RemoveOutcome::Removed { next_focus: b }
        );
        assert_eq!(root, LayoutNode::pane(b));
    }

    #[test]
    fn remove_keeps_surviving_split_identity() {
        // 收起父节点后， surviving 子树必须沿用原 SplitId，否则其可拖拽尺寸会重置。
        let (mut root, ids) = chain(3, SplitAxis::Horizontal);
        let inner_id = match &root {
            LayoutNode::Split { first, .. } => match **first {
                LayoutNode::Split { id, .. } => id,
                _ => panic!("链式布局的左孩子应为分割节点"),
            },
            _ => panic!("三窗格链式布局根节点应为分割节点"),
        };
        root.remove_leaf(ids[2]);
        match &root {
            LayoutNode::Split { id, .. } => assert_eq!(*id, inner_id),
            _ => panic!("剩余两窗格仍应有分割节点"),
        }
    }

    #[test]
    fn remove_last_pane_empties_the_group() {
        let only = tab();
        let mut root = LayoutNode::pane(only);
        assert_eq!(root.remove_leaf(only), RemoveOutcome::Empty);
    }

    #[test]
    fn toggle_axis_flips_only_the_pane_own_cut() {
        let (mut root, [a, b, c]) = nested();
        // H(a, V(b, c))：翻转 a 只动根节点，内层上下分屏保持原样。
        assert!(root.toggle_axis(a));
        match &root {
            LayoutNode::Split { axis, second, .. } => {
                assert_eq!(*axis, SplitAxis::Vertical);
                assert_eq!(second.root_axis(), SplitAxis::Vertical);
            }
            _ => panic!("根节点应保持为分割节点"),
        }
        // 翻转 b 只动它所在的内层切分。
        assert!(root.toggle_axis(b));
        match &root {
            LayoutNode::Split { axis, second, .. } => {
                assert_eq!(*axis, SplitAxis::Vertical);
                assert_eq!(second.root_axis(), SplitAxis::Horizontal);
            }
            _ => panic!("根节点应保持为分割节点"),
        }
        assert_eq!(root.pane_ids(), vec![a, b, c]);
        assert!(!root.toggle_axis(tab()));
    }

    #[test]
    fn focus_neighbor_walks_nested_splits_in_the_requested_direction() {
        let (root, [a, b, c]) = nested();
        assert_eq!(root.focus_neighbor(a, PaneDirection::Right), Some(b));
        assert_eq!(root.focus_neighbor(b, PaneDirection::Left), Some(a));
        assert_eq!(root.focus_neighbor(b, PaneDirection::Down), Some(c));
        assert_eq!(root.focus_neighbor(c, PaneDirection::Up), Some(b));
        // a 占满整个左侧高度：它的切分是水平的，外层再无上下切分，向下无处可去。
        assert_eq!(root.focus_neighbor(a, PaneDirection::Down), None);
        assert_eq!(root.focus_neighbor(c, PaneDirection::Down), None);
        // c 向右：内层是上下切分，外层已到边（c 在后侧），无窗格可去。
        assert_eq!(root.focus_neighbor(c, PaneDirection::Right), None);
        assert_eq!(root.focus_neighbor(a, PaneDirection::Left), None);
        assert_eq!(root.focus_neighbor(tab(), PaneDirection::Right), None);
    }

    #[test]
    fn focus_neighbor_across_three_column_row_returns_adjacent_column() {
        let (root, ids) = chain(3, SplitAxis::Horizontal);
        // 链式 H(H(a, b), c)：从 a 向右应命中 b，而不是越过 b 直达 c。
        assert_eq!(
            root.focus_neighbor(ids[0], PaneDirection::Right),
            Some(ids[1])
        );
        assert_eq!(
            root.focus_neighbor(ids[1], PaneDirection::Right),
            Some(ids[2])
        );
        assert_eq!(
            root.focus_neighbor(ids[2], PaneDirection::Left),
            Some(ids[1])
        );
        assert_eq!(root.focus_neighbor(ids[2], PaneDirection::Right), None);
    }

    #[test]
    fn focus_neighbor_needs_at_least_two_panes() {
        let only = tab();
        let root = LayoutNode::pane(only);
        assert_eq!(root.focus_neighbor(only, PaneDirection::Right), None);
    }

    #[test]
    fn saved_round_trip_preserves_nesting_and_order() {
        let (root, ids) = nested_chain();
        let shape = root.to_saved();
        assert_eq!(LayoutNode::saved_pane_count(&shape), Some(3));
        let mut cursor = ids.iter().copied();
        let rebuilt = LayoutNode::from_saved(&shape, &mut cursor).unwrap();
        assert_eq!(rebuilt.pane_ids(), ids);
        assert_eq!(rebuilt.root_axis(), SplitAxis::Horizontal);
    }

    #[test]
    fn from_saved_rejects_bad_shapes() {
        // 叶子数超过上限：损坏或恶意设置不得一次拉起过多连接。
        let huge = chain_saved(MAX_PANES + 1, SplitAxis::Horizontal);
        let mut ids = (0..MAX_PANES + 2).map(|_| tab());
        assert_eq!(LayoutNode::from_saved(&huge, &mut ids), None);

        // 可用窗格标识不足时整棵拒绝，而不是留下半棵树。
        let shape = chain_saved(3, SplitAxis::Horizontal);
        let mut one = std::iter::once(tab());
        assert_eq!(LayoutNode::from_saved(&shape, &mut one), None);
    }

    #[test]
    fn chain_shape_counts_leaves_and_keeps_axis() {
        let single = chain_saved(1, SplitAxis::Horizontal);
        assert_eq!(single, SavedLayout::Pane);
        assert_eq!(LayoutNode::saved_pane_count(&single), Some(1));
        let vertical = chain_saved(3, SplitAxis::Vertical);
        assert_eq!(LayoutNode::saved_pane_count(&vertical), Some(3));
        let mut cursor = (0..3).map(|_| tab());
        let root = LayoutNode::from_saved(&vertical, &mut cursor).unwrap();
        assert_eq!(root.root_axis(), SplitAxis::Vertical);
        assert_eq!(root.pane_count(), 3);
    }

    /// 与 `nested()` 同形但走持久化通道构造，用于验证形状往返。
    fn nested_chain() -> (LayoutNode, Vec<TabId>) {
        let ids = vec![tab(), tab(), tab()];
        let shape = SavedLayout::Split {
            axis: SplitAxis::Horizontal,
            first: Box::new(SavedLayout::Pane),
            second: Box::new(SavedLayout::Split {
                axis: SplitAxis::Vertical,
                first: Box::new(SavedLayout::Pane),
                second: Box::new(SavedLayout::Pane),
            }),
        };
        let mut cursor = ids.iter().copied();
        (LayoutNode::from_saved(&shape, &mut cursor).unwrap(), ids)
    }

    #[test]
    fn contains_matches_only_real_panes() {
        let (root, ids) = chain(3, SplitAxis::Vertical);
        assert!(root.contains(ids[1]));
        assert!(!root.contains(tab()));
    }
}
