//! Git 侧栏实体与事件定义。
//!
//! 设计原理（Why）：
//! - 与 `ContainerSidebarView` 同一口径，侧栏视图本身只读 `GitHub` 快照渲染，
//!   不直接依赖应用外壳；点击、拖拽、右键菜单全部通过 `GitSidebarEvent` 上报，
//!   由外层桥接在渲染期统一执行。

use crate::app::AppState;
use crate::git::types::{GitDropTarget, GitHub, GitSidebarDrag};
use gpui_kit::App;
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
///
/// 设计原理（Why）：项目目录、徽标与调度状态和侧栏同生命周期，统一由本实体
/// 持有；标签页关闭不会丢失注册表，窗口关闭时则随侧栏一次性释放。
pub(crate) struct GitSidebarView {
    pub(crate) state: GitHub,
}

impl EventEmitter<GitSidebarEvent> for GitSidebarView {}

impl Default for GitSidebarView {
    fn default() -> Self {
        Self::new(GitHub::default())
    }
}

impl GitSidebarView {
    pub(crate) fn new(state: GitHub) -> Self {
        Self { state }
    }
}

impl AppState {
    /// 读取 Git 全局注册表；只暴露 Git 域状态，不开放整个应用状态。
    pub(crate) fn git_registry<'a>(&self, cx: &'a App) -> &'a GitHub {
        &self.git_sidebar.read(cx).state
    }

    /// 原子更新 Git 全局注册表；回调结束即释放实体借用。
    pub(crate) fn update_git_registry<R>(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut GitHub) -> R,
    ) -> R {
        self.git_sidebar
            .update(cx, |view, _| update(&mut view.state))
    }
}
