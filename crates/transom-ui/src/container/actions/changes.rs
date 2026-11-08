//! 容器文件系统变更检查动作。

use super::*;

impl ContainerWorkspace {
    /// 确保当前选中容器的文件系统变更已加载。
    ///
    /// 设计原理（Why）：变更归属工作区实体，缓存命中判断与在途请求拦截都在
    /// 实体内部完成；渲染函数拿到句柄后直接调用，不再经外壳转手。
    pub(crate) fn ensure_container_changes(&mut self) {
        let Some(container_id) = self.state.selected_container().map(|c| c.id.clone()) else {
            return;
        };
        let already_loaded = self
            .state
            .changes
            .as_ref()
            .map(|(id, _)| id == &container_id)
            .unwrap_or(false);
        if !already_loaded && !self.state.changes_loading {
            self.refresh_container_changes();
        }
    }

    /// 重新拉取当前选中容器的文件系统变更。
    pub(crate) fn refresh_container_changes(&mut self) {
        let Some(container_id) = self.state.selected_container().map(|c| c.id.clone()) else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.state.changes_loading = true;
        let previous = self.state.changes_request.replace(request_id);
        if let Some(previous) = previous {
            self.cancel_container_request(previous);
        }
        log::info!("正在拉取容器文件系统变更，容器: {container_id}，请求: {request_id}");
        self.send_tracked(
            request_id,
            ContainerCommand::DiffContainer {
                tab_id: self.tab_id,
                request_id,
                container: container_id,
            },
        );
    }
}
