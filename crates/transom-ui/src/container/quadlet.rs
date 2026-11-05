//! Quadlet 单元弹窗状态与 systemd 用户单元操作。

use super::*;

impl AppShell {
    /// 打开 Quadlet 弹窗并扫描宿主单元。
    pub(crate) fn open_container_quadlets_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.quadlets_dialog_open = true;
            workspace.quadlets = Vec::new();
            workspace.quadlets_error = None;
            workspace.quadlets_loading = true;
        }
        self.refresh_container_quadlets(tab_id);
        cx.notify();
    }

    pub(crate) fn close_container_quadlets_dialog(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.quadlets_dialog_open = false;
            workspace.quadlets_request = None;
            workspace.quadlets_action_request = None;
            workspace.quadlets_loading = false;
        }
        cx.notify();
    }

    pub(crate) fn refresh_container_quadlets(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        if !workspace.quadlets_dialog_open || workspace.quadlets_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.quadlets_request = Some(request_id);
            workspace.quadlets_loading = true;
            workspace.quadlets_error = None;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ListQuadlets { tab_id, request_id },
        );
    }

    pub(crate) fn container_quadlet_action(
        &mut self,
        tab_id: TabId,
        unit: String,
        action: transom_core::ContainerQuadletAction,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        if workspace.quadlets_request.is_some() || workspace.quadlets_action_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.quadlets_action_request = Some(request_id);
            workspace.quadlets_loading = true;
            workspace.quadlets_error = None;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::QuadletAction {
                tab_id,
                request_id,
                unit,
                action,
            },
        );
        cx.notify();
    }

    pub(super) fn on_container_quadlets_listed(
        &mut self,
        request_id: ContainerRequestId,
        units: Vec<transom_core::ContainerQuadletUnit>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
            return;
        };
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if workspace.quadlets_request != Some(request_id) || !workspace.quadlets_dialog_open {
                return;
            }
            workspace.quadlets = units;
            workspace.quadlets_request = None;
            workspace.quadlets_loading = false;
            workspace.quadlets_error = None;
        }
        cx.notify();
    }
}
