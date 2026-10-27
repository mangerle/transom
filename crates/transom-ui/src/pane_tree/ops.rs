//! 分割树操作算法：切分窗格、摘除窗格、翻转切分轴与跨分屏焦点移动。

use super::types::{LayoutNode, MAX_PANES, PaneDirection, RemoveOutcome, is_leaf};
use transom_core::{SplitAxis, TabId};

impl LayoutNode {
    /// 把 `target` 窗格沿 `axis` 切成两半，新窗格落在其同轴后侧（右或下）。
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
    pub(crate) fn remove_leaf(&mut self, target: TabId) -> RemoveOutcome {
        if matches!(self, Self::Pane(_)) {
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
    pub(crate) fn focus_neighbor(&self, from: TabId, direction: PaneDirection) -> Option<TabId> {
        if self.pane_count() < 2 || !self.contains(from) {
            return None;
        }
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
}
