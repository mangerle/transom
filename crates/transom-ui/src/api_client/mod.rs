//! 桌面 API 客户端：项目侧栏、项目工作区标签与窗口级动作桥接。
//!
//! 设计原理（Why）：左侧活动栏只切换单例项目侧栏；每个打开的项目由
//! [`ApiWorkspace`] 实体独占请求树、编辑器和响应状态，并通过事件把需要窗口或
//! 持久化的意图交回外壳。这样 API 客户端完全复用 Transom 现有的导航、标签、
//! 并排与关闭模型，不引入第二个主界面状态源。

mod actions;
mod sidebar;
mod sidebar_view;
mod workspace;
mod workspace_view;

pub(crate) use actions::{
    on_sidebar_event, on_sidebar_request, on_workspace_event, on_workspace_request,
};
pub(crate) use sidebar::{ApiSidebarEvent, ApiSidebarState, ApiSidebarView};
pub(crate) use sidebar_view::{ApiSidebarParams, render_api_sidebar};
pub(crate) use workspace::{ApiWorkspace, ApiWorkspaceEvent};
pub(crate) use workspace_view::{ApiWorkspaceParams, render_api_workspace};
