//! Git 侧栏事件在 AppShell 侧的消费桥接。

use crate::app::{AppShell, AppState};
use crate::git::sidebar::GitSidebarEvent;
use gpui_kit::*;

/// 消费 Git 侧栏上报的意图事件并调度对应的跨模块动作。
pub(crate) fn on_git_sidebar_event(
    app_state: &mut AppState,
    event: &GitSidebarEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitSidebarEvent::OpenNewFolderModal => {
            crate::git::actions::modal_bridges::open_git_folder_modal(app_state, window, cx);
        }
        GitSidebarEvent::OpenCloneModal => {
            crate::git::actions::modal_bridges::open_git_clone_modal(app_state, window, cx);
        }
        GitSidebarEvent::OpenProjectModal(folder) => {
            crate::git::actions::modal_bridges::open_git_project_modal(
                app_state, *folder, window, cx,
            );
        }
        GitSidebarEvent::RefreshAll => {
            crate::git::actions::GitActions::new(app_state).refresh_all_git_projects(cx);
        }
        GitSidebarEvent::ToggleFolderCollapsed(folder_id) => {
            if app_state.update_git_registry(cx, |git| git.toggle_folder_collapsed(*folder_id)) {
                crate::git::actions::GitActions::new(app_state).persist_git_projects(cx);
            }
        }
        GitSidebarEvent::Drop(drag, target) => {
            if app_state.update_git_registry(cx, |git| git.apply_drop(*drag, *target)) {
                crate::git::actions::GitActions::new(app_state).persist_git_projects(cx);
            } else {
                cx.notify();
            }
        }
        GitSidebarEvent::SelectProject(id) => {
            if app_state.update_git_registry(cx, |git| git.select_project(*id)) {
                cx.notify();
            }
        }
        GitSidebarEvent::OpenProjectTab(project) => {
            crate::git::actions::GitActions::new(app_state).open_git_tab(
                project.clone(),
                window,
                cx,
            );
        }
        GitSidebarEvent::OpenInTerminal(id) => {
            let path = app_state
                .git_registry(cx)
                .projects
                .iter()
                .find(|item| item.id == *id)
                .map(|item| item.path.clone());
            if let Some(path) = path {
                app_state.ui.window_requests.push_local_terminal(Some(path));
            }
        }
        GitSidebarEvent::RefreshProject(id) => {
            if let Some(project) = app_state
                .git_registry(cx)
                .projects
                .iter()
                .find(|item| item.id == *id)
                .cloned()
            {
                crate::git::actions::GitActions::new(app_state).git_refresh(&project, cx);
            }
        }
        GitSidebarEvent::MoveProjectToRoot(id) => {
            if app_state.update_git_registry(cx, |git| git.move_project_to_root(*id)) {
                crate::git::actions::GitActions::new(app_state).persist_git_projects(cx);
            } else {
                cx.notify();
            }
        }
        GitSidebarEvent::RemoveProject(id) => {
            crate::git::actions::GitActions::new(app_state).remove_git_project(*id, cx);
        }
        GitSidebarEvent::OpenRenameFolderModal(id) => {
            crate::git::actions::modal_bridges::open_git_rename_folder_modal(
                app_state, *id, window, cx,
            );
        }
        GitSidebarEvent::DeleteFolder(id) => {
            app_state.update_git_registry(cx, |git| git.delete_folder(*id));
            crate::git::actions::GitActions::new(app_state).persist_git_projects(cx);
        }
    }
}
