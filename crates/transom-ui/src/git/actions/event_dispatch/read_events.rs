//! Git 读取与日志/差异事件落地。

use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{ErrorCode, GitAppEvent};

/// 处理读取类、日志分页、提交详情与差异加载事件。
pub(super) fn handle_read_event(
    shell: &mut AppShell,
    event: GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
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
        } => handle_log_loaded(shell, project_id, request_id, *page, cx),
        GitAppEvent::RebaseTodoLoaded {
            project_id,
            request_id,
            base,
            steps,
        } => {
            drop_workspace_request(shell, project_id, request_id, cx);
            crate::git::actions::modal_bridges::apply_git_rebase_todo(shell, &base, steps, cx);
        }
        GitAppEvent::CommitUrlLoaded {
            project_id,
            request_id,
            oid,
            url,
        } => {
            drop_workspace_request(shell, project_id, request_id, cx);
            if let Some(msg) = crate::git::web_link::resolve_commit_url_feedback(url, &oid) {
                shell.set_transient_status(msg, cx);
            }
            cx.notify();
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
            drop_workspace_request(shell, project_id, request_id, cx);
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
        } => handle_read_failed(shell, project_id, request_id, error, cx),
        _ => {}
    }
}

fn drop_workspace_request(
    shell: &mut AppShell,
    project_id: transom_core::GitProjectId,
    request_id: transom_core::GitRequestId,
    cx: &mut Context<AppShell>,
) {
    if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
        workspace.update(cx, |workspace, _| {
            workspace.state.drop_request(request_id);
        });
    }
}

fn handle_log_loaded(
    shell: &mut AppShell,
    project_id: transom_core::GitProjectId,
    request_id: transom_core::GitRequestId,
    page: transom_core::LogPage,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) else {
        return;
    };
    let Some((landed, keep_waiting, jump, tab_id)) = workspace.update(cx, |workspace, _| {
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
    }) else {
        return;
    };
    cx.notify();
    if let Some(index) = landed {
        if let Some(workspace) = shell.state.git_workspace(tab_id) {
            workspace.update(cx, |ws, cx| ws.select_commit_by_index(index, cx));
        }
    } else if keep_waiting {
        // 目标行还没加载：顺着分页往下翻，翻到头或超页数上限才提示找不到。
        if let Some(workspace) = shell.state.git_workspace(tab_id) {
            workspace.update(cx, |ws, cx| ws.load_more_log(cx));
        }
    } else if let Some(target) = jump {
        log::info!("Git 提交跳转未落地即停止，目标: {target:?}");
        shell.set_transient_status(rust_i18n::t!("git.log.jump_missing"), cx);
        cx.notify();
    }
}

fn handle_read_failed(
    shell: &mut AppShell,
    project_id: transom_core::GitProjectId,
    request_id: transom_core::GitRequestId,
    error: transom_core::AppError,
    cx: &mut Context<AppShell>,
) {
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
    shell.state.update_git_registry(cx, |git| {
        let badge = git.badges.entry(project_id).or_default();
        badge.loading = false;
        badge.error = Some(message.clone());
    });
    if !not_repo {
        shell.set_error(message);
    }
    cx.notify();
}
