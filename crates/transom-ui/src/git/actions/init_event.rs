//! Git 仓库补初始化事件落地处理。

use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{AppError, GitRequestId};

/// 面板内补初始化的结果：成功刷新状态与日志，失败把原因落回标签页。
pub(super) fn apply_git_project_init_event(
    shell: &mut AppShell,
    request_id: GitRequestId,
    path: &std::path::Path,
    error: Option<AppError>,
    cx: &mut Context<AppShell>,
) {
    let Some((pending_id, project_id)) = shell.state.git_registry(cx).pending_project_init else {
        return;
    };
    if pending_id != request_id {
        return;
    }
    shell
        .state
        .update_git_registry(cx, |git| git.pending_project_init = None);
    let project = shell
        .state
        .git_registry(cx)
        .projects
        .iter()
        .find(|project| project.id == project_id)
        .cloned();
    match error {
        Some(error) => {
            let message = localized_error(&error);
            log::error!(
                "初始化 Git 仓库失败，项目: {project_id}，路径: {}，原因: {message}",
                path.display()
            );
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.init_running = false;
                    state.not_repo = true;
                    state.status_loading = false;
                    state.status_error = Some(message.clone());
                    state.log.loading = false;
                    state.log.error = Some(message.clone());
                });
            }
            shell.state.update_git_registry(cx, |git| {
                let badge = git.badges.entry(project_id).or_default();
                badge.loading = false;
                badge.error = Some(message.clone());
            });
            shell.set_error(message);
        }
        None => {
            log::info!(
                "目录已初始化为 Git 仓库，项目: {project_id}，路径: {}",
                path.display()
            );
            if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    let state = &mut workspace.state;
                    state.init_running = false;
                    state.not_repo = false;
                    state.status_error = None;
                    state.log.loading = false;
                    state.log.error = None;
                });
            }
            shell.state.update_git_registry(cx, |git| {
                let badge = git.badges.entry(project_id).or_default();
                badge.loading = false;
                badge.error = None;
            });
            if let Some(project) = project {
                crate::git::actions::git_refresh(shell, &project, cx);
                if let Some(workspace) = shell.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |ws, cx| ws.load_log_notified(0, cx));
                }
            }
        }
    }
    cx.notify();
}
