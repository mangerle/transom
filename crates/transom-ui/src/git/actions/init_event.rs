//! Git 仓库补初始化事件落地处理。

use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{AppError, GitRequestId};

impl AppShell {
    /// 面板内补初始化的结果：成功刷新状态与日志，失败把原因落回标签页。
    pub(super) fn apply_git_project_init_event(
        &mut self,
        request_id: GitRequestId,
        path: &std::path::Path,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        let Some((pending_id, project_id)) = self.state.git.pending_project_init else {
            return;
        };
        if pending_id != request_id {
            return;
        }
        self.state.git.pending_project_init = None;
        let project = self
            .state
            .git
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
                if let Some(tab_id) = self.git_tab_for_project(project_id)
                    && let Some(state) = self.git_state_of_tab_mut(tab_id)
                {
                    state.init_running = false;
                    state.not_repo = true;
                    state.status_loading = false;
                    state.status_error = Some(message.clone());
                    state.log.loading = false;
                    state.log.error = Some(message.clone());
                }
                let badge = self.state.git.badges.entry(project_id).or_default();
                badge.loading = false;
                badge.error = Some(message.clone());
                self.set_error(message);
            }
            None => {
                log::info!(
                    "目录已初始化为 Git 仓库，项目: {project_id}，路径: {}",
                    path.display()
                );
                if let Some(tab_id) = self.git_tab_for_project(project_id)
                    && let Some(state) = self.git_state_of_tab_mut(tab_id)
                {
                    state.init_running = false;
                    state.not_repo = false;
                    state.status_error = None;
                    state.log.loading = false;
                    state.log.error = None;
                }
                let badge = self.state.git.badges.entry(project_id).or_default();
                badge.loading = false;
                badge.error = None;
                if let Some(project) = project {
                    self.git_refresh(&project, cx);
                    if let Some(tab_id) = self.git_tab_for_project(project_id) {
                        self.git_load_log(tab_id, 0, cx);
                    }
                }
            }
        }
        cx.notify();
    }
}
