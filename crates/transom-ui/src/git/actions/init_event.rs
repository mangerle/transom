//! Git 仓库补初始化事件落地处理。

use crate::app::{AppShell, AppState};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{AppError, GitRequestId};

/// 面板内补初始化的结果：成功刷新状态与日志，失败把原因落回标签页。
pub(super) fn apply_git_project_init_event(
    app_state: &mut AppState,
    request_id: GitRequestId,
    path: &std::path::Path,
    error: Option<AppError>,
    cx: &mut Context<AppShell>,
) {
    let Some(project_id) =
        app_state.update_git_registry(cx, |git| git.take_project_init(request_id))
    else {
        return;
    };
    let project = app_state
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
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_project_init_result(Some(&message));
                });
            }
            app_state
                .update_git_registry(cx, |git| git.set_badge_result(project_id, Some(&message)));
            app_state.set_error(message);
        }
        None => {
            log::info!(
                "目录已初始化为 Git 仓库，项目: {project_id}，路径: {}",
                path.display()
            );
            if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                workspace.update(cx, |workspace, _| {
                    workspace.apply_project_init_result(None);
                });
            }
            app_state.update_git_registry(cx, |git| git.set_badge_result(project_id, None));
            if let Some(project) = project {
                crate::git::actions::GitActions::new(app_state).git_refresh(&project, cx);
                if let Some(workspace) = app_state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |ws, cx| ws.load_log_notified(0, cx));
                }
            }
        }
    }
    cx.notify();
}
