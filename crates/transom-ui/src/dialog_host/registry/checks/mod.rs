//! 弹窗打开判定与脏检查文本项：按领域拆分，统一由注册表引用。
//!
//! 每个函数只服务注册表里的一个条目，签名统一为 `fn(&AppShell, &App)`。
//! 新增弹窗时在对应领域文件补一对函数，再在 [`super::MODALS`] 登记一行。

mod container;
mod git;
mod session;

pub(super) use container::*;
pub(super) use git::*;
pub(super) use session::*;

use crate::app::AppShell;
use gpui_kit::App;

/// 不参与脏判定的弹窗共用空实现。
pub(super) fn no_values(_: &AppShell, _: &App) -> Vec<String> {
    Vec::new()
}
