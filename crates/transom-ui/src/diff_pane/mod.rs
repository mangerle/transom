//! 公共 Diff 渲染与计算底座。
//!
//! 统一 Git 差异视图与 SFTP 远程文件编辑保存前预览的底层算法、显示选项、
//! 工具条交互与 `gpui_kit::component::diff::Diff` 渲染口径。

mod compute;
mod controls;
mod types;

pub(crate) use compute::{
    create_diff_state, prepare_diff_document, sync_diff_state_options,
};
pub(crate) use controls::{
    OptionsUpdater, render_change_nav_buttons, render_diff_body, render_option_controls,
};
pub(crate) use types::{DiffViewOptions, IgnoreWhitespace};
