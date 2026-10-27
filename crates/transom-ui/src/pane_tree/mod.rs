//! 标签组内的分割树：窗格布局的唯一事实来源。
//!
//! 采用二叉分割树结构维护分屏布局，统一收敛切分、摘除、翻转与焦点寻径逻辑。

mod ops;
mod saved;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use types::{LayoutNode, MAX_PANES, PaneDirection, RemoveOutcome};
