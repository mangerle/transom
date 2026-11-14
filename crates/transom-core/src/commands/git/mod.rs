//! Git 领域命令与事件。
//!
//! 将 Git 面板与后台 git 引擎之间的命令与事件收敛为独立子系统契约。

mod command;
mod event;

pub use command::GitCommand;
pub use event::{GitAppEvent, GitShelfOperation};
