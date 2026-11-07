//! 引用栏的分支树构建与渲染。
//!
//! 设计原理（Why）：IDEA 的引用栏不是平铺列表。本地分支按 `/` 分目录，远程分支
//! 先按远端名分组，再按分支路径分层；当前分支和远端默认分支优先显示。模型与渲染
//! 分层维护，确保筛选、排序和视觉行为可以分别测试与调整。

mod model;
mod view;

pub(crate) use model::RefGroup;
pub(crate) use view::render_ref_group;
