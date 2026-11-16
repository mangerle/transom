//! Git 读取与日志/差异事件落地。

use crate::app::{AppShell, AppState};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{ErrorCode, GitAppEvent};

/// 处理读取类、日志分页、提交详情与差异加载事件。
pub(super) fn handle_read_event(
    app_state: &mut AppState,
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
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_refs_loaded(branches, tags, paths);
                });
            }
            cx.notify();
        }
        GitAppEvent::LogLoaded {
            project_id,
            request_id,
            page,
        } => handle_log_loaded(app_state, project_id, request_id, *page, cx),
        GitAppEvent::RebaseTodoLoaded {
            project_id,
            request_id,
            base,
            steps,
        } => {
            drop_workspace_request(app_state, project_id, request_id, cx);
            crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                .apply_git_rebase_todo(&base, steps, cx);
        }
        GitAppEvent::CommitUrlLoaded {
            project_id,
            request_id,
            oid,
            url,
        } => {
            drop_workspace_request(app_state, project_id, request_id, cx);
            if let Some(msg) = crate::git::web_link::resolve_commit_url_feedback(url, &oid) {
                app_state.set_transient_status(msg, cx);
            }
            cx.notify();
        }
        GitAppEvent::CommitFilesLoaded {
            project_id,
            request_id,
            oid,
            files,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_commit_files_loaded(request_id, &files);
                });
            }
            crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                .apply_git_push_commit_files(&oid, files, cx);
            cx.notify();
        }
        GitAppEvent::OutgoingCommitsLoaded {
            project_id,
            request_id,
            branch,
            remote_branch,
            commits,
        } => {
            drop_workspace_request(app_state, project_id, request_id, cx);
            crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                .apply_git_outgoing_commits(project_id, &branch, &remote_branch, commits, cx);
            cx.notify();
        }
        GitAppEvent::DiffLoaded {
            project_id,
            request_id,
            diff,
        } => {
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
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
        } => handle_read_failed(app_state, project_id, request_id, error, cx),
        _ => {}
    }
}

fn drop_workspace_request(
    app_state: &mut AppState,
    project_id: transom_core::GitProjectId,
    request_id: transom_core::GitRequestId,
    cx: &mut Context<AppShell>,
) {
    if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
        workspace.update(cx, |workspace, _| {
            workspace.state.drop_request(request_id);
        });
    }
}

fn handle_log_loaded(
    app_state: &mut AppState,
    project_id: transom_core::GitProjectId,
    request_id: transom_core::GitRequestId,
    page: transom_core::LogPage,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) else {
        return;
    };
    let Some(resolution) = workspace.update(cx, |workspace, _| {
        workspace.apply_log_loaded(request_id, page)
    }) else {
        return;
    };
    let tab_id = workspace.read(cx).tab_id;
    cx.notify();
    if let Some(index) = resolution.landed {
        if let Some(workspace) = app_state.git_workspace(tab_id) {
            workspace.update(cx, |ws, cx| ws.select_commit_by_index(index, cx));
        }
    } else if resolution.keep_waiting {
        // 目标行还没加载：顺着分页往下翻，翻到头或超页数上限才提示找不到。
        if let Some(workspace) = app_state.git_workspace(tab_id) {
            workspace.update(cx, |ws, cx| ws.load_more_log(cx));
        }
    } else if let Some(target) = resolution.missing_jump {
        log::info!("Git 提交跳转未落地即停止，目标: {target:?}");
        app_state.set_transient_status(rust_i18n::t!("git.log.jump_missing"), cx);
        cx.notify();
    }
}

fn handle_read_failed(
    app_state: &mut AppState,
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
    if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
        workspace.update(cx, |workspace, _| {
            workspace.apply_read_failure(request_id, &message, not_repo);
        });
    }
    app_state.update_git_registry(cx, |git| git.set_badge_result(project_id, Some(&message)));
    if !not_repo {
        app_state.set_error(message);
    }
    cx.notify();
}
