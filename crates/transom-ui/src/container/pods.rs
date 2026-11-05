//! Podman Pod 列表弹窗与启停操作。

use super::*;

impl AppShell {
    /// 打开 Pod 弹窗并拉取列表；仅 Podman CLI 连接可用。
    pub(crate) fn open_container_pods_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.pods_dialog_open = true;
            workspace.pods = Vec::new();
            workspace.pods_error = None;
            workspace.pods_loading = true;
        }
        self.refresh_container_pods(tab_id);
        cx.notify();
    }

    /// 关闭 Pod 弹窗；在途列表请求结果到达后会被丢弃。
    pub(crate) fn close_container_pods_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.pods_dialog_open = false;
            workspace.pods_request = None;
            workspace.pods_action_request = None;
            workspace.pods_loading = false;
        }
        cx.notify();
    }

    /// 刷新 Pod 列表；已有在途请求时直接复用。
    pub(crate) fn refresh_container_pods(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        if !workspace.pods_dialog_open || workspace.pods_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.pods_request = Some(request_id);
            workspace.pods_loading = true;
            workspace.pods_error = None;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ListPods { tab_id, request_id },
        );
    }

    /// 对 Pod 执行启停/重启/删除，完成后自动刷新列表。
    pub(crate) fn container_pod_action(
        &mut self,
        tab_id: TabId,
        pod: String,
        action: transom_core::ContainerPodAction,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        if workspace.pods_request.is_some() || workspace.pods_action_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.pods_action_request = Some(request_id);
            workspace.pods_loading = true;
            workspace.pods_error = None;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::PodAction {
                tab_id,
                request_id,
                pod,
                action,
            },
        );
        cx.notify();
    }

    /// 处理 Pod 列表响应。
    pub(super) fn on_container_pods_listed(
        &mut self,
        request_id: ContainerRequestId,
        pods: Vec<transom_core::ContainerPodSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
            return;
        };
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if workspace.pods_request != Some(request_id) || !workspace.pods_dialog_open {
                return;
            }
            workspace.pods = pods;
            workspace.pods_request = None;
            workspace.pods_loading = false;
            workspace.pods_error = None;
        }
        cx.notify();
    }
}
