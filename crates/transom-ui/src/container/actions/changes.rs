//! 容器文件系统变更检查动作。

use super::*;

impl AppShell {
    /// 确保当前选中容器的文件系统变更已加载。
    pub(crate) fn ensure_container_changes(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let Some(container_id) = workspace.selected_container().map(|c| c.id.clone()) else {
            return;
        };
        let already_loaded = workspace
            .changes
            .as_ref()
            .map(|(id, _)| id == &container_id)
            .unwrap_or(false);
        if !already_loaded && !workspace.changes_loading {
            self.refresh_container_changes(tab_id);
        }
    }

    /// 重新拉取当前选中容器的文件系统变更。
    pub(crate) fn refresh_container_changes(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container_id) = workspace.selected_container().map(|c| c.id.clone()) else {
            return;
        };
        workspace.changes_loading = true;
        let request_id = ContainerRequestId::new();
        if let Some(previous) = workspace.changes_request.replace(request_id) {
            self.cancel_container_request(tab_id, previous);
        }
        log::info!("正在拉取容器文件系统变更，容器: {container_id}，请求: {request_id}");
        let command = ContainerCommand::DiffContainer {
            tab_id,
            request_id,
            container: container_id,
        };
        self.send_tracked(tab_id, request_id, command);
    }
}
