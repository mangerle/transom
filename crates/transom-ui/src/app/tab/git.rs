//! Git 仓库工作区标签。

use super::{TabContent, WorkspaceTab};
use crate::git::GitWorkspace;
use gpui_kit::Entity;
use transom_core::TabId;

/// Git 标签只持有仓库工作区实体，项目路径与分支状态由实体维护。
///
/// 设计原理（Why）：Git 项目不是终端会话，项目身份和状态都来自
/// [`GitWorkspace`]；使用专用标签内容后，标签不会再被 SFTP、转发或连接事件
/// 误识别，同时仍可参与拖拽、并排和关闭。
pub(crate) struct GitTab {
    pub(crate) workspace: Entity<GitWorkspace>,
}

impl WorkspaceTab {
    /// 构造 Git 工作区标签。
    pub(crate) fn new_git(id: TabId, title: String, workspace: Entity<GitWorkspace>) -> Self {
        Self {
            id,
            title,
            status: super::TabStatus::Connected,
            content: TabContent::Git(GitTab { workspace }),
        }
    }

    /// 获取 Git 工作区实体句柄。
    pub(crate) fn git_workspace(&self) -> Option<Entity<GitWorkspace>> {
        match &self.content {
            TabContent::Git(git) => Some(git.workspace.clone()),
            _ => None,
        }
    }
}
