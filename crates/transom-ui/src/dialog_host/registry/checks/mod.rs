//! 弹窗打开判定与脏检查文本项：按领域拆分，统一由注册表引用。
//!
//! 每个函数只服务注册表里的一个条目，签名统一为 `fn(&DialogCheckState, &App)`。
//! 新增弹窗时在对应领域文件补一对函数，再在 [`super::MODALS`] 登记一行。

mod container;
mod git;
mod session;

pub(super) use container::*;
pub(super) use git::*;
pub(super) use session::*;

use super::DialogCheckState;
use gpui_kit::App;

/// 不参与脏判定的弹窗共用空实现。
pub(super) fn no_values(_: &DialogCheckState, _: &App) -> Vec<String> {
    Vec::new()
}

/// 当前激活标签页为 API 工作区且新建/导入请求弹窗已打开时返回 `true`。
pub(super) fn api_new_request_open(state: &DialogCheckState, cx: &App) -> bool {
    state
        .workspace
        .active_tab
        .and_then(|tab_id| state.workspace.tab(tab_id))
        .and_then(|tab| tab.api_workspace())
        .is_some_and(|workspace| workspace.read(cx).is_new_request_modal_open())
}

/// 当前激活标签页为 API 工作区且环境与全局配置弹窗已打开时返回 `true`。
pub(super) fn api_config_open(state: &DialogCheckState, cx: &App) -> bool {
    state
        .workspace
        .active_tab
        .and_then(|tab_id| state.workspace.tab(tab_id))
        .and_then(|tab| tab.api_workspace())
        .is_some_and(|workspace| workspace.read(cx).is_config_modal_open())
}
