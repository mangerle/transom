//! 容器文件系统变更检查动作。

use super::*;

impl AppShell {
    /// 确保当前选中容器的文件系统变更已加载。
    pub(crate) fn ensure_container_changes(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((_container_id, already_loaded, loading)) = self
            .with_container(tab_id, cx, |workspace| {
                let container_id = workspace.selected_container().map(|c| c.id.clone())?;
                let already_loaded = workspace
                    .changes
                    .as_ref()
                    .map(|(id, _)| id == &container_id)
                    .unwrap_or(false);
                Some((container_id, already_loaded, workspace.changes_loading))
            })
            .flatten()
        else {
            return;
        };
        if !already_loaded && !loading {
            self.refresh_container_changes(tab_id, cx);
        }
    }

    /// 重新拉取当前选中容器的文件系统变更。
    pub(crate) fn refresh_container_changes(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(container_id) = self
            .with_container(tab_id, cx, |workspace| {
                workspace.selected_container().map(|c| c.id.clone())
            })
            .flatten()
        else {
            return;
        };
        let request_id = ContainerRequestId::new();
        let previous = self
            .with_container_mut(tab_id, cx, |workspace| {
                workspace.changes_loading = true;
                workspace.changes_request.replace(request_id)
            })
            .flatten();
        if let Some(previous) = previous {
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
