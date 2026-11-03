//! 容器日志采样：拉取、跟随、行数与时间戳切换。

use super::*;

impl AppShell {
    pub(crate) fn ensure_container_logs(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.logs_request.is_some() {
            return;
        }
        let follow = workspace.logs_follow;
        let tail = workspace.logs_tail;
        let timestamps = workspace.logs_timestamps;
        workspace.logs.clear();
        let request_id = ContainerRequestId::new();
        workspace.logs_request = Some(request_id);

        if workspace.section == ContainerSection::Compose {
            let Some(project) = workspace.selected_compose() else {
                return;
            };
            let project_name = project.name.clone();
            let member_containers: Vec<String> = workspace
                .containers
                .iter()
                .filter(|c| c.compose_project.as_deref() == Some(&project_name))
                .map(|c| c.name.clone())
                .collect();
            self.send_tracked(
                tab_id,
                request_id,
                ContainerCommand::ProjectLogs {
                    tab_id,
                    request_id,
                    project: project_name,
                    containers: member_containers,
                    tail,
                    follow,
                    timestamps,
                },
            );
        } else {
            let Some(container) = workspace.selected_container_name() else {
                return;
            };
            self.send_tracked(
                tab_id,
                request_id,
                ContainerCommand::Logs {
                    tab_id,
                    request_id,
                    container,
                    tail,
                    follow,
                    timestamps,
                },
            );
        }
    }

    /// 重开日志流：跟随、尾部行数与时间戳都要改才能生效，先取消旧流再拉新流。
    fn restart_container_logs(&mut self, tab_id: TabId) {
        if let Some(request_id) = self
            .state
            .container_workspace_mut(tab_id)
            .and_then(|workspace| workspace.logs_request.take())
        {
            self.cancel_container_request(tab_id, request_id);
        }
        self.ensure_container_logs(tab_id);
    }

    /// 切换日志尾部行数档位（0 表示全量）。
    pub(crate) fn set_container_logs_tail(&mut self, tab_id: TabId, tail: usize) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if workspace.logs_tail == tail {
                return;
            }
            workspace.logs_tail = tail;
        }
        self.restart_container_logs(tab_id);
    }

    /// 开关日志时间戳：由引擎附带时间戳，界面不做二次格式化。
    pub(crate) fn toggle_container_logs_timestamps(&mut self, tab_id: TabId) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.logs_timestamps = !workspace.logs_timestamps;
        }
        self.restart_container_logs(tab_id);
    }

    /// 把当前日志缓冲整体复制到剪贴板，便于贴到工单或聊天里。
    pub(crate) fn copy_container_logs(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(text) = self
            .state
            .container_workspace(tab_id)
            .map(|workspace| workspace.logs.clone())
        else {
            return;
        };
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.notify();
    }

    pub(crate) fn toggle_container_logs_follow(&mut self, tab_id: TabId, follow: bool) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.logs_follow = follow;
        }
        self.restart_container_logs(tab_id);
    }
}
