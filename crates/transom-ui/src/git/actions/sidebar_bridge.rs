//! Git 侧栏事件在 AppShell 侧的消费桥接。

use crate::app::AppShell;
use crate::git::sidebar::GitSidebarEvent;
use gpui_kit::*;

/// 消费 Git 侧栏上报的意图事件并调度对应的跨模块动作。
pub(crate) fn on_git_sidebar_event(
    shell: &mut AppShell,
    event: &GitSidebarEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitSidebarEvent::OpenNewFolderModal => {
            crate::git::actions::modal_bridges::open_git_folder_modal(shell, window, cx);
        }
        GitSidebarEvent::OpenCloneModal => {
            crate::git::actions::modal_bridges::open_git_clone_modal(shell, window, cx);
        }
        GitSidebarEvent::OpenProjectModal(folder) => {
            crate::git::actions::modal_bridges::open_git_project_modal(
                shell, *folder, window, cx,
            );
        }
        GitSidebarEvent::RefreshAll => {
            crate::git::actions::refresh_all_git_projects(shell, cx);
        }
        GitSidebarEvent::ToggleFolderCollapsed(folder_id) => {
            crate::git::actions::toggle_git_folder_collapsed(shell, *folder_id, cx);
        }
        GitSidebarEvent::Drop(drag, target) => {
            crate::git::actions::apply_git_drop(shell, *drag, *target, cx);
        }
        GitSidebarEvent::SelectProject(id) => {
            crate::git::actions::select_git_project(shell, *id, cx);
        }
        GitSidebarEvent::OpenProjectTab(project) => {
            crate::git::actions::open_git_tab(shell, project.clone(), window, cx);
        }
        GitSidebarEvent::OpenInTerminal(id) => {
            let path = shell
                .state
                .git
                .projects
                .iter()
                .find(|item| item.id == *id)
                .map(|item| item.path.clone());
            if let Some(path) = path {
                crate::connection::spawn_local_terminal(shell, Some(path), cx);
            }
        }
        GitSidebarEvent::RefreshProject(id) => {
            if let Some(project) = shell
                .state
                .git
                .projects
                .iter()
                .find(|item| item.id == *id)
                .cloned()
            {
                crate::git::actions::git_refresh(shell, &project, cx);
            }
        }
        GitSidebarEvent::MoveProjectToRoot(id) => {
            crate::git::actions::git_move_project_to_root(shell, *id, cx);
        }
        GitSidebarEvent::RemoveProject(id) => {
            crate::git::actions::remove_git_project(shell, *id, cx);
        }
        GitSidebarEvent::OpenRenameFolderModal(id) => {
            crate::git::actions::modal_bridges::open_git_rename_folder_modal(
                shell, *id, window, cx,
            );
        }
        GitSidebarEvent::DeleteFolder(id) => {
            crate::git::actions::delete_git_folder(shell, *id, cx);
        }
    }
}
