//! Podman Pod 列表弹窗与启停操作。

use super::*;

impl AppShell {
    /// 打开 Pod 弹窗并拉取列表；仅 Podman CLI 连接可用。
    pub(crate) fn open_container_pods_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.pods_dialog_open = true;
            workspace.pods = Vec::new();
            workspace.pods_error = None;
            workspace.pods_loading = true;
        });
        self.refresh_container_pods(tab_id, cx);
        cx.notify();
    }

    /// 关闭 Pod 弹窗；在途列表请求结果到达后会被丢弃。
    pub(crate) fn close_container_pods_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.pods_dialog_open = false;
            workspace.pods_request = None;
            workspace.pods_action_request = None;
            workspace.pods_loading = false;
        });
        cx.notify();
    }

    /// 刷新 Pod 列表；已有在途请求时直接复用。
    pub(crate) fn refresh_container_pods(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let active = self
            .with_container(tab_id, cx, |workspace| {
                workspace.pods_dialog_open && workspace.pods_request.is_none()
            })
            .unwrap_or(false);
        if !active {
            return;
        }
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.pods_request = Some(request_id);
            workspace.pods_loading = true;
            workspace.pods_error = None;
        });
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::ListPods { tab_id, request_id },
                );
            });
        }
    }

    /// 对 Pod 执行启停/重启/删除，完成后自动刷新列表。
    pub(crate) fn container_pod_action(
        &mut self,
        tab_id: TabId,
        pod: String,
        action: transom_core::ContainerPodAction,
        cx: &mut Context<Self>,
    ) {
        let idle = self
            .with_container(tab_id, cx, |workspace| {
                workspace.pods_request.is_none() && workspace.pods_action_request.is_none()
            })
            .unwrap_or(false);
        if !idle {
            return;
        }
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.pods_action_request = Some(request_id);
            workspace.pods_loading = true;
            workspace.pods_error = None;
        });
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::PodAction {
                        tab_id,
                        request_id,
                        pod,
                        action,
                    },
                );
            });
        }
        cx.notify();
    }

    /// 处理 Pod 列表响应。
    pub(super) fn on_container_pods_listed(
        &mut self,
        request_id: ContainerRequestId,
        pods: Vec<transom_core::ContainerPodSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
            return;
        };
        let applied = self.with_container_mut(tab_id, cx, |workspace| {
            if workspace.pods_request != Some(request_id) || !workspace.pods_dialog_open {
                return false;
            }
            workspace.pods = pods;
            workspace.pods_request = None;
            workspace.pods_loading = false;
            workspace.pods_error = None;
            true
        });
        if applied == Some(true) {
            cx.notify();
        }
    }
}
