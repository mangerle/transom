//! Git 事件落地：把控制器回传的事件写进对应标签页与侧栏徽标。
//!
//! 设计原理（Why）：事件处理是「唯一写界面的入口」，单独成模块便于逐条核对
//! 迟到请求的丢弃口径（按 `request_id` 比对）与写动作后的自动刷新，
//! 不让这些规则散落在各个点击回调里。
//!
//! 状态写入一律走 `Entity<GitWorkspace>::update`：闭包内只改该标签页的状态并把
//! 外壳后续需要的数据带出来，闭包结束后再调用外壳方法，避免同一实体被二次租借。

use crate::app::AppShell;
use crate::git::GitCommitTab;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{ErrorCode, GitAppEvent};

/// Git 事件落地。
pub(crate) fn dispatch_git_event(
    shell: &mut AppShell,
    event: GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitAppEvent::StatusLoaded { project_id, status } => {
            action_done::apply_git_status(shell, project_id, *status, cx);
        }
        GitAppEvent::RefsLoaded {
            project_id,
            branches,
            tags,
            paths,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.branches = branches;
                    state.tags = tags;
                    state.tracked_paths = paths;
                });
            }
            cx.notify();
        }
        GitAppEvent::LogLoaded {
            project_id,
            request_id,
            page,
        } => {
            let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) else {
                return;
            };
            let Some((landed, keep_waiting, jump, tab_id)) =
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    let (held, appended) = state.log.request?;
                    if held != request_id {
                        // 迟到的旧请求：界面已经发起了新一轮加载，直接丢弃。
                        log::debug!("丢弃迟到的 Git 日志结果，请求: {request_id}");
                        return None;
                    }
                    state.log.request = None;
                    state.log.loading = false;
                    state.log.error = None;
                    let has_more = page.has_more;
                    if appended {
                        state.log.append_entries(&page.entries);
                    } else {
                        state.log.replace_entries(page.entries);
                    }
                    state.log.has_more = has_more;
                    let jump = state.log.jump.clone();
                    let landed = jump
                        .as_ref()
                        .and_then(|target| target.resolve(&state.log.entries));
                    let within_budget = state.log.jump_pages < crate::git::types::MAX_JUMP_PAGES;
                    let keep_waiting =
                        landed.is_none() && jump.is_some() && state.log.has_more && within_budget;
                    if landed.is_some() {
                        state.log.jump = None;
                        state.log.jump_pages = 0;
                    } else if keep_waiting {
                        state.log.jump_pages += 1;
                    }
                    Some((landed, keep_waiting, jump, workspace.tab_id))
                })
            else {
                return;
            };
            cx.notify();
            if let Some(index) = landed {
                crate::git::actions::git_select_commit_by_index(shell, tab_id, index, cx);
            } else if keep_waiting {
                // 目标行还没加载：顺着分页往下翻，翻到头或超页数上限才提示找不到。
                crate::git::actions::git_load_more_log(shell, tab_id, cx);
            } else if let Some(target) = jump {
                log::info!("Git 提交跳转未落地即停止，目标: {target:?}");
                shell.set_transient_status(rust_i18n::t!("git.log.jump_missing"), cx);
                cx.notify();
            }
        }
        GitAppEvent::RebaseTodoLoaded {
            project_id,
            request_id,
            base,
            steps,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.state.drop_request(request_id);
                });
            }
            crate::git::actions::modal_bridges::apply_git_rebase_todo(shell, &base, steps, cx);
        }
        GitAppEvent::CommitUrlLoaded {
            project_id,
            request_id,
            oid,
            url,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.state.drop_request(request_id);
                });
            }
            crate::git::web_link::git_open_commit_url(shell, url, &oid, cx);
        }
        GitAppEvent::CommitFilesLoaded {
            project_id,
            request_id,
            oid,
            files,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    if state.amend_request == Some(request_id) {
                        state.amend_files = files.clone();
                        state.amend_files_loading = false;
                        state.amend_request = None;
                    } else {
                        state.commit_files = files.clone();
                        state.commit_files_loading = false;
                    }
                });
            }
            crate::git::actions::modal_bridges::apply_git_push_commit_files(shell, &oid, files, cx);
            cx.notify();
        }
        GitAppEvent::OutgoingCommitsLoaded {
            project_id,
            request_id,
            branch,
            remote_branch,
            commits,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.state.drop_request(request_id);
                });
            }
            crate::git::actions::modal_bridges::apply_git_outgoing_commits(
                shell,
                project_id,
                &branch,
                &remote_branch,
                commits,
                cx,
            );
            cx.notify();
        }
        GitAppEvent::DiffLoaded {
            project_id,
            request_id,
            diff,
        } => {
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, cx| {
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    state.apply_diff_loaded(request_id, *diff);
                    workspace.schedule_diff_prepare(request_id, cx);
                });
            }

            cx.notify();
        }
        GitAppEvent::ReadFailed {
            project_id,
            request_id,
            error,
        } => {
            let message = localized_error(&error);
            let not_repo = error.code == ErrorCode::GitNotARepo;
            if not_repo {
                log::debug!("Git 项目尚未初始化为仓库: {project_id}");
            } else {
                log::error!("Git 读取失败，项目: {project_id}，原因: {message}");
            }
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.diff_tasks.remove(&request_id);
                    let state = &mut workspace.state;
                    state.drop_request(request_id);
                    // 差异请求失败按 request_id 精确落到对应槽位与标签，避免误伤其他标签与日志状态
                    if state.apply_diff_failed(request_id, &message) {
                        return;
                    }
                    state.log.loading = false;
                    state.log.error = Some(message.clone());
                    state.commit_files_loading = false;
                    if not_repo {
                        // 不是仓库是「可以补初始化」的正常状态，不只是错误：
                        // 工作区据此显示初始化入口，而不是只剩一句红字。
                        state.not_repo = true;
                        state.status_loading = false;
                        state.status_error = Some(message.clone());
                    }
                });
            }
            let badge = shell.state.git.badges.entry(project_id).or_default();
            badge.loading = false;
            badge.error = Some(message.clone());
            if !not_repo {
                shell.set_error(message);
            }
            cx.notify();
        }
        GitAppEvent::ActionDone {
            project_id,
            action,
            error,
        } => {
            action_done::handle_git_action_done(shell, project_id, action, error, cx);
        }
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
                super::git_refresh(shell, &project, cx);
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
                super::git_refresh(shell, &project, cx);
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
        GitAppEvent::Probed { .. } => {
            crate::git::project_modal::apply_git_probe_event(shell, event, cx);
        }
        GitAppEvent::Inited {
            request_id,
            path,
            error,
        } => {
            let ours = shell
                .state
                .git
                .pending_project_init
                .is_some_and(|(pending_id, _)| pending_id == request_id);
            if ours {
                super::init_event::apply_git_project_init_event(
                    shell, request_id, &path, error, cx,
                );
            } else {
                crate::git::project_modal::apply_git_probe_event(
                    shell,
                    GitAppEvent::Inited {
                        request_id,
                        path,
                        error,
                    },
                    cx,
                );
            }
        }
        GitAppEvent::CloneProgress { .. } | GitAppEvent::CloneDone { .. } => {
            crate::git::clone_modal::apply_git_clone_event(shell, event, cx);
        }
        GitAppEvent::ConflictSidesLoaded {
            project_id,
            path,
            sides,
            ..
        } => {
            crate::git::actions::modal_bridges::apply_git_conflict_sides(
                shell, project_id, &path, *sides, cx,
            );
        }
    }
}

mod action_done;
