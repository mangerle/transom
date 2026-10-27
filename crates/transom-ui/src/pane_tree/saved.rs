//! 分割树持久化形状的显式栈解析与安全还原算法。

use super::types::{LayoutNode, MAX_PANES};
use transom_core::{SavedLayout, SplitAxis, TabId};

impl LayoutNode {
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
}
