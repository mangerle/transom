//! Git 侧栏实体与事件定义。
//!
//! 设计原理（Why）：
//! - 与 `ContainerSidebarView` 同一口径，侧栏视图本身只读 `GitHub` 快照渲染，
//!   不直接依赖应用外壳；点击、拖拽、右键菜单全部通过 `GitSidebarEvent` 上报，
//!   由外层桥接在渲染期统一执行。

use crate::git::types::{GitDropTarget, GitSidebarDrag};
use gpui_kit::*;
use transom_core::{GitFolderId, GitProject, GitProjectId};

/// Git 侧栏向外壳发出的意图事件。
#[derive(Clone)]
pub(crate) enum GitSidebarEvent {
    OpenNewFolderModal,
    OpenCloneModal,
    OpenProjectModal(Option<GitFolderId>),
    RefreshAll,
    ToggleFolderCollapsed(GitFolderId),
    Drop(GitSidebarDrag, GitDropTarget),
    SelectProject(GitProjectId),
    OpenProjectTab(GitProject),
    OpenInTerminal(GitProjectId),
    RefreshProject(GitProjectId),
    MoveProjectToRoot(GitProjectId),
    RemoveProject(GitProjectId),
    OpenRenameFolderModal(GitFolderId),
    DeleteFolder(GitFolderId),
}

/// Git 侧栏视图实体。
pub(crate) struct GitSidebarView;

impl EventEmitter<GitSidebarEvent> for GitSidebarView {}

impl Default for GitSidebarView {
    fn default() -> Self {
        Self::new()
    }
}

impl GitSidebarView {
    pub(crate) fn new() -> Self {
        Self
    }
}
