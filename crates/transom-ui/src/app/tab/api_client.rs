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

impl WorkspaceTab {
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

    /// 在现有 API 标签内切换绑定的项目与工作区实体。
    pub(crate) fn replace_api_client(
        &mut self,
        title: String,
        project_id: ApiProjectId,
        workspace: Entity<ApiWorkspace>,
    ) {
        self.title = title;
        if let TabContent::ApiClient(api) = &mut self.content {
            api.project_id = project_id;
            api.workspace = workspace;
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
}
