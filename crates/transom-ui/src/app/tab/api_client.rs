//! API 客户端工作区标签。

use super::{TabContent, WorkspaceTab};
use crate::api_client::ApiWorkspace;
use gpui_kit::Entity;
use transom_api::ApiProjectId;
use transom_core::TabId;

/// API 项目标签保存项目标识与工作区实体。
///
/// 设计原理（Why）：API 请求编辑、响应和发送任务都随项目标签存活；公共标签
/// 只保留栏位元数据，API 标签不会伪造终端会话，也不会进入 SFTP、转发、重连等
/// 会话专属路径。
pub(crate) struct ApiClientTab {
    pub(crate) project_id: ApiProjectId,
    pub(crate) workspace: Entity<ApiWorkspace>,
}

/// API 客户端首页；不绑定项目，只承载项目选择和新建/导入入口。
pub(crate) struct ApiHomeTab;

impl WorkspaceTab {
    /// 构造 API 客户端无项目首页标签。
    pub(crate) fn new_api_home(id: TabId) -> Self {
        Self {
            id,
            title: rust_i18n::t!("api.home_title").to_string(),
            status: super::TabStatus::Connected,
            content: TabContent::ApiHome(ApiHomeTab),
        }
    }

    /// 构造 API 客户端项目标签。
    pub(crate) fn new_api_client(
        id: TabId,
        title: String,
        project_id: ApiProjectId,
        workspace: Entity<ApiWorkspace>,
    ) -> Self {
        Self {
            id,
            title,
            status: super::TabStatus::Connected,
            content: TabContent::ApiClient(Box::new(ApiClientTab {
                project_id,
                workspace,
            })),
        }
    }

    /// 获取 API 工作区实体句柄。
    pub(crate) fn api_workspace(&self) -> Option<Entity<ApiWorkspace>> {
        match &self.content {
            TabContent::ApiClient(api) => Some(api.workspace.clone()),
            _ => None,
        }
    }

    /// 获取 API 项目标识；非 API 标签返回 `None`。
    pub(crate) fn api_project_id(&self) -> Option<ApiProjectId> {
        match &self.content {
            TabContent::ApiClient(api) => Some(api.project_id),
            _ => None,
        }
    }

    /// 是否为 API 客户端首页标签。
    pub(crate) fn is_api_home(&self) -> bool {
        matches!(&self.content, TabContent::ApiHome(_))
    }
}
