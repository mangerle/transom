//! Git 工作树、暂存栈（Stash）与搁置（Shelf）事件落地。

use crate::app::{AppShell, AppState};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::GitAppEvent;

/// 处理工作树、暂存栈与搁置相关事件。
pub(super) fn handle_shelf_worktree_event(
    app_state: &mut AppState,
    event: GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitAppEvent::WorktreesLoaded {
            project_id,
            request_id,
            entries,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_worktrees_loaded(request_id, entries);
                });
            }
            cx.notify();
        }
        GitAppEvent::StashesLoaded {
            project_id,
            request_id,
            entries,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_stashes_loaded(request_id, entries);
                });
            }
            cx.notify();
        }
        GitAppEvent::StashFilesLoaded {
            project_id,
            request_id,
            selector,
            files,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_stash_files_loaded(request_id, &selector, files);
                });
            }
            cx.notify();
        }
        GitAppEvent::ShelvesLoaded {
            project_id,
            request_id,
            shelves,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_shelves_loaded(request_id, shelves);
                });
            }
            cx.notify();
        }
        GitAppEvent::Shelved {
            project_id,
            request_id,
            entry,
        } => {
            let project = app_state
                .git_workspace_of_project(project_id, cx)
                .map(|workspace| {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_shelved(request_id, entry)
                    })
                });
            app_state.set_transient_status(rust_i18n::t!("git.shelf.create"), cx);
            if let Some(project) = project {
                crate::git::actions::GitActions::new(app_state).git_refresh(&project, cx);
            }
            cx.notify();
        }
        GitAppEvent::ShelfApplied {
            project_id,
            request_id: _,
            shelf_id,
            pop,
            cleanup_error,
        } => {
            // 补丁已应用是既成事实；删除条目失败只影响列表，不能按整次失败处理，
            // 也不能把仍留在存储里的条目从界面移除（否则用户没法重试删除）。
            let cleanup_failed = cleanup_error.is_some();
            let project = app_state
                .git_workspace_of_project(project_id, cx)
                .map(|workspace| {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_shelf_applied(&shelf_id, pop, cleanup_failed)
                    })
                });
            let key = if pop && !cleanup_failed {
                "git.shelf.pop"
            } else {
                "git.shelf.apply"
            };
            app_state.set_transient_status(rust_i18n::t!(key), cx);
            if let Some(error) = cleanup_error {
                let message = localized_error(&error);
                log::warn!("搁置项补丁已应用，但删除条目失败，项目: {project_id}，原因: {message}");
                app_state.set_error(message);
            }
            if let Some(project) = project {
                crate::git::actions::GitActions::new(app_state).git_refresh(&project, cx);
            }
            cx.notify();
        }
        GitAppEvent::ShelfDropped {
            project_id,
            request_id: _,
            shelf_id,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_shelf_dropped(&shelf_id);
                });
            }
            app_state.set_transient_status(rust_i18n::t!("git.shelf.drop"), cx);
            cx.notify();
        }
        GitAppEvent::ShelfFailed {
            project_id,
            request_id,
            operation,
            error,
        } => {
            let message = localized_error(&error);
            log::error!(
                "Git 搁置操作失败，操作: {operation:?}，项目: {project_id}，原因: {message}"
            );
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.drop_shelf_request(request_id);
                });
            }
            app_state.set_error(message);
            cx.notify();
        }
        _ => {}
    }
}
