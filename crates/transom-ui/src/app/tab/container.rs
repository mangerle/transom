//! 容器引擎工作区标签。

use super::{TabContent, WorkspaceTab};
use crate::container::ContainerWorkspace;
use gpui_kit::Entity;
use transom_core::{ContainerEngineProfile, TabId};

/// 容器标签保存引擎配置与工作区实体；连接状态由工作区实体作为唯一真相源。
///
/// 设计原理（Why）：容器工作区已有独立连接状态机和请求生命周期，不应再包装成
/// 终端 `Session`。标签只保留引擎配置与实体句柄，关闭时由 `TabCloseAction`
/// 明确发送容器断开命令。
pub(crate) struct ContainerTab {
    pub(crate) profile: ContainerEngineProfile,
    pub(crate) workspace: Entity<ContainerWorkspace>,
}

impl WorkspaceTab {
    /// 构造容器工作区标签。
    pub(crate) fn new_container(
        id: TabId,
        title: String,
        profile: ContainerEngineProfile,
        workspace: Entity<ContainerWorkspace>,
    ) -> Self {
        Self {
            id,
            title,
            status: super::TabStatus::Connecting,
            content: TabContent::Containers(Box::new(ContainerTab { profile, workspace })),
        }
    }

    /// 获取容器工作区实体句柄。
    pub(crate) fn container_workspace(&self) -> Option<Entity<ContainerWorkspace>> {
        match &self.content {
            TabContent::Containers(container) => Some(container.workspace.clone()),
            _ => None,
        }
    }

    /// 获取容器引擎配置。
    pub(crate) fn container_profile(&self) -> Option<&ContainerEngineProfile> {
        match &self.content {
            TabContent::Containers(container) => Some(&container.profile),
            _ => None,
        }
    }
}
