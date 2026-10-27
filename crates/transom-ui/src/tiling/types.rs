//! 并排网格数据契约、节点树与条目类型定义。

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use transom_core::{GroupId, SplitAxis, SplitId};

/// 单个并排视图允许的最大格子数。
pub(crate) const MAX_TILING_SLOTS: usize = 16;

/// 并排网格中的一个节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TilingNode {
    Slot(usize),
    Stack {
        id: SplitId,
        axis: SplitAxis,
        children: Vec<TilingNode>,
    },
}

impl TilingNode {
    /// 全部分割节点标识，供渲染侧清理已消失的拖拽状态。
    pub(crate) fn split_ids(&self) -> Vec<SplitId> {
        let mut out = Vec::with_capacity(MAX_TILING_SLOTS);
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            if let Self::Stack { id, children, .. } = node {
                out.push(*id);
                stack.extend(children.iter());
            }
        }
        out
    }
}

/// 并排组的运行期标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TilingId(u64);

impl TilingId {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for TilingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// 标签栏里的一个可见条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StripEntry {
    /// 独立标签。
    Single(GroupId),
    /// 并排组的合并条目：`id` 定位并排组，`focus` 是条目上展示的焦点标签。
    Tiled { id: TilingId, focus: GroupId },
}

/// 布局选择：自适应或手选网格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TilingLayoutChoice {
    Auto,
    Grid { rows: usize, cols: usize },
}
