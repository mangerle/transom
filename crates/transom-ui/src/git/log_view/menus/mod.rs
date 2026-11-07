//! 日志工作区的右键上下文菜单模块。
//!
//! 设计原理（Why）：
//! - 将原来单个巨型菜单文件按操作领域清晰拆解为过滤条、变更文件树、提交表格行和引用树；
//! - 各菜单均完整对照 IntelliJ IDEA 交互与文案体系（图 1 ~ 图 5），并在右侧以 IDE 同款弱色显示快捷键；
//! - 单文件严格控制在 300 行以内，杜绝巨型单体文件。

mod commit_menu;
mod file_menu;
mod filter_menus;
mod local_branch_menu;
mod remote_branch_menu;
mod tag_menu;

pub(crate) use commit_menu::{CommitMenuTarget, render_commit_menu};
pub(crate) use file_menu::render_file_tree_menu;
pub(crate) use filter_menus::{render_date_menu, render_path_menu};
pub(crate) use local_branch_menu::{LocalBranchMenuTarget, render_local_branch_menu};
pub(crate) use remote_branch_menu::{RemoteBranchMenuTarget, render_remote_branch_menu};
pub(crate) use tag_menu::{TagMenuTarget, render_tag_menu};
