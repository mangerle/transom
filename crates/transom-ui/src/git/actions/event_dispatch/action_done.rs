//! Git 写动作完成事件落地与状态快照刷新。

use crate::app::AppShell;
use crate::git::commit_modal::worktree_diff_request;
use crate::git::types::first_worktree_diff;
use crate::palette::localized_error;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{GitAction, GitCommand, GitProjectId};

pub(super) fn handle_git_action_done(
    shell: &mut AppShell,
    project_id: GitProjectId,
    action: GitAction,
    error: Option<transom_core::AppError>,
    cx: &mut Context<AppShell>,
) {
    let message = error.as_ref().map(localized_error);
    match &message {
        Some(text) => {
            log::error!("Git 动作失败，动作: {action:?}，项目: {project_id}，原因: {text}");
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |ws, _| ws.clear_patch_on_failure(action));
            }
            crate::git::actions::modal_bridges::fail_git_rebase_modal(
                shell,
                project_id,
                action,
                text.clone(),
                cx,
            );
            shell.set_error(text.clone());
        }
        None => {
            // 补丁动作要报出落盘路径，其余动作沿用动作名文案。
            let saved = shell
                .state
                .git_workspace_of_project(project_id, cx)
                .and_then(|workspace| workspace.update(cx, |ws, _| ws.take_patch_done_status()));
            shell.set_transient_status(
                saved.unwrap_or_else(|| rust_i18n::t!(action.i18n_key()).to_string()),
                cx,
            );
        }
    }
    if matches!(action, GitAction::AddWorktree) && message.is_none() {
        // 新建成功才把目录登记成项目：失败时留下一个打不开的条目只会碍事。
        let pending = shell
            .state
            .git_workspace_of_project(project_id, cx)
            .and_then(|workspace| {
                workspace.update(cx, |workspace, _| workspace.state.pending_worktree.take())
            });
        if let Some((path, _checkout)) = pending {
            crate::git::actions::add_git_project(
                shell,
                crate::git::GitProjectDraft::new(std::path::PathBuf::from(&path)),
                cx,
            );
        }
    }
    // 工作树的新建/删除/锁定都会改动 `git worktree list` 的结果：
    // 不重取就会把已删掉的目录继续摆在列表里，点它只会报错。
    if matches!(
        action,
        GitAction::AddWorktree
            | GitAction::RemoveWorktree
            | GitAction::LockWorktree
            | GitAction::UnlockWorktree
    ) && message.is_none()
    {
        let worktree_project = shell
            .state
            .git_registry(cx)
            .projects
            .iter()
            .find(|item| item.id == project_id)
            .cloned();
        if let Some(project) = worktree_project {
            if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
                workspace.update(cx, |ws, cx| ws.reload_worktrees(cx));
            } else {
                shell.send_command(GitCommand::ListWorktrees {
                    project,
                    request_id: transom_core::GitRequestId::new(),
                });
            }
        }
    }
    // 写动作后一律刷新：索引与工作区都可能被 git 顺带改写（如提交后
    // ahead 变化、worktree add 后分支增加）。
    if let Some(project) = shell
        .state
        .git_registry(cx)
        .projects
        .iter()
        .find(|item| item.id == project_id)
        .cloned()
    {
        crate::git::actions::git_refresh(shell, &project, cx);
    }
    if matches!(
        action,
        GitAction::RebaseRun | GitAction::Rebase | GitAction::RebaseAbort
    ) && message.is_none()
    {
        // 变基改写了整段历史：日志必须重取，否则界面还停在旧提交上。
        // 失败时不关对话框，用户要看到原因并可以改清单重试。
        if matches!(action, GitAction::RebaseRun) {
            crate::git::actions::modal_bridges::close_git_rebase_modal(shell, cx);
        }
        if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
            workspace.update(cx, |ws, cx| ws.load_log_notified(0, cx));
        }
    }
    if matches!(action, GitAction::Push) {
        crate::git::actions::modal_bridges::handle_git_push_result(
            shell,
            project_id,
            message.clone(),
            cx,
        );
    }
    if matches!(
        action,
        GitAction::ResolveConflictOurs
            | GitAction::ResolveConflictTheirs
            | GitAction::ResolveConflictMark
    ) {
        crate::git::actions::modal_bridges::handle_git_conflict_resolve_done(
            shell,
            message.clone(),
            cx,
        );
    }
    if matches!(action, GitAction::Commit)
        && let Some(tab_id) = crate::git::actions::git_tab_for_project(shell, project_id, cx)
    {
        let mut should_push = false;
        if let Some(workspace) = shell.state.git_workspace(tab_id) {
            workspace.update(cx, |workspace, cx| {
                let state = &mut workspace.state;
                state.commit_submitting = false;
                if message.is_none() {
                    state.checked = HashSet::new();
                    state.commit_amend = false;
                    state.commit_clear_pending =
                        crate::app::app_settings(cx).git.clear_message_on_commit;
                    // 让下一次状态刷新重新选中第一个改动并拉差异。
                    state.diff = None;
                    state.last_diff = None;
                    should_push = std::mem::take(&mut state.push_after_commit);
                }
            });
        }
        if message.is_none() {
            if let Some(workspace) = shell.state.git_workspace(tab_id) {
                workspace.update(cx, |ws, cx| ws.load_log_notified(0, cx));
            }
            if should_push
                && let Some(project) = shell
                    .state
                    .git_registry(cx)
                    .projects
                    .iter()
                    .find(|item| item.id == project_id)
                    .cloned()
            {
                let show_dialog = crate::app::app_settings(cx).git.push_dialog;
                if show_dialog {
                    crate::git::actions::modal_bridges::open_git_push_modal(
                        shell, &project, None, cx,
                    );
                } else {
                    shell.send_command(GitCommand::Push {
                        project,
                        options: transom_core::GitPushOptions::default(),
                    });
                }
            }
        }
    }
    cx.notify();
}

/// 把状态快照同时写进侧栏徽标与标签页。
pub(super) fn apply_git_status(
    shell: &mut AppShell,
    project_id: GitProjectId,
    status: transom_core::RepoStatus,
    cx: &mut Context<AppShell>,
) {
    let dirty = status.entries.len();
    let branch = match &status.head {
        transom_core::HeadState::OnBranch(name) => Some(name.clone()),
        transom_core::HeadState::Initial { branch } => Some(branch.clone()),
        transom_core::HeadState::Detached => status
            .oid
            .as_deref()
            .map(|oid| oid.chars().take(7).collect()),
    };
    shell.state.update_git_registry(cx, |git| {
        let badge = git.badges.entry(project_id).or_default();
        badge.branch = branch;
        badge.dirty = dirty;
        badge.ahead = status.ahead;
        badge.behind = status.behind;
        badge.loading = false;
        badge.error = None;
    });
    if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
        workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.status_error = None;
            state.status_loading = false;
            state.not_repo = false;
            state.status = Some(status);
        });
    }
    // 提交页首屏直接给第一个改动的差异：用户不用先点一下才知道右侧能看什么。
    if let Some(tab_id) = crate::git::actions::git_tab_for_project(shell, project_id, cx) {
        let first = shell
            .state
            .git_workspace(tab_id)
            .map(|workspace| {
                let state = &workspace.read(cx).state;
                state
                    .diff
                    .is_none()
                    .then(|| state.status.as_ref().and_then(first_worktree_diff))
                    .flatten()
            })
            .unwrap_or_default();
        if let Some((group, path)) = first
            && let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx)
        {
            workspace.update(cx, |ws, cx| {
                ws.open_diff(worktree_diff_request(group, path), cx);
            });
        } else if let Some(workspace) = shell.state.git_workspace(tab_id) {
            workspace.update(cx, |ws, cx| ws.refresh_active_worktree_diff(cx));
        }
    }
    cx.notify();
}
