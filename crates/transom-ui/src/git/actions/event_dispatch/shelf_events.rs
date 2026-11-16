//! Git 工作树、暂存栈（Stash）与搁置（Shelf）事件落地。

use crate::app::AppShell;
use crate::git::GitCommitTab;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::GitAppEvent;

/// 处理工作树、暂存栈与搁置相关事件。
pub(super) fn handle_shelf_worktree_event(
    shell: &mut AppShell,
    event: GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitAppEvent::WorktreesLoaded {
            project_id,
            request_id,
            entries,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    state.worktrees = entries;
                });
            }
            cx.notify();
        }
        GitAppEvent::StashesLoaded {
            project_id,
            request_id,
            entries,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    state.stashes = entries;
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
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    state.stash_files_loading = false;
                    // 迟到的清单不能写进已经换过展开项的视图：编号会随删除前移，
                    // 对不上选择器就如实丢弃，而不是让界面挂着一份别人的文件。
                    if state.expanded_stash.as_deref() == Some(selector.as_str()) {
                        state.stash_files = files;
                    }
                });
            }
            cx.notify();
        }
        GitAppEvent::ShelvesLoaded {
            project_id,
            request_id,
            shelves,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    state.shelves = shelves;
                    // 展开项可能已在别处删除：保留指向不存在条目的展开态会让面板空白。
                    if state
                        .expanded_shelf
                        .as_ref()
                        .is_some_and(|id| !state.shelves.iter().any(|entry| &entry.id == id))
                    {
                        state.expanded_shelf = None;
                    }
                });
            }
            cx.notify();
        }
        GitAppEvent::Shelved {
            project_id,
            request_id,
            entry,
        } => {
            let project = shell
                .state
                .git_workspace_of_project(project_id, cx)
                .map(|workspace| {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        // 列表刷新事件可能已带回同一条目：按 ID 去重后再插到最前。
                        state.shelves.retain(|shelf| shelf.id != entry.id);
                        state.shelves.insert(0, entry);
                        state.checked.clear();
                        state.commit_tab = GitCommitTab::Shelf;
                        state.project.clone()
                    })
                });
            shell.set_transient_status(rust_i18n::t!("git.shelf.create"), cx);
            if let Some(project) = project {
                crate::git::actions::GitActions::new(&mut shell.state).git_refresh(&project, cx);
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
            let project = shell
                .state
                .git_workspace_of_project(project_id, cx)
                .map(|workspace| {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        if pop && !cleanup_failed {
                            state.shelves.retain(|shelf| shelf.id != shelf_id);
                            if state.expanded_shelf.as_deref() == Some(shelf_id.as_str()) {
                                state.expanded_shelf = None;
                            }
                        }
                        state.project.clone()
                    })
                });
            let key = if pop && !cleanup_failed {
                "git.shelf.pop"
            } else {
                "git.shelf.apply"
            };
            shell.set_transient_status(rust_i18n::t!(key), cx);
            if let Some(error) = cleanup_error {
                let message = localized_error(&error);
                log::warn!("搁置项补丁已应用，但删除条目失败，项目: {project_id}，原因: {message}");
                shell.set_error(message);
            }
            if let Some(project) = project {
                crate::git::actions::GitActions::new(&mut shell.state).git_refresh(&project, cx);
            }
            cx.notify();
        }
        GitAppEvent::ShelfDropped {
            project_id,
            request_id: _,
            shelf_id,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.shelves.retain(|shelf| shelf.id != shelf_id);
                    if state.expanded_shelf.as_deref() == Some(shelf_id.as_str()) {
                        state.expanded_shelf = None;
                    }
                });
            }
            shell.set_transient_status(rust_i18n::t!("git.shelf.drop"), cx);
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
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.state.drop_request(request_id);
                });
            }
            shell.set_error(message);
            cx.notify();
        }
        _ => {}
    }
}
