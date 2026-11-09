//! Podman Quadlet 单元弹窗状态与 systemd 用户单元操作。
//!
//! 动作归属（Why）：打开、关闭、刷新与单元操作只读写本标签页状态并投递命令，
//! 全部落在 [`ContainerWorkspace`] 上；外壳只负责把窗口级意图转成实体更新。

use super::*;

impl ContainerWorkspace {
    /// 打开 Quadlet 弹窗并扫描宿主单元。
    pub(crate) fn open_quadlets_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.quadlets_dialog_open = true;
        self.state.quadlets = Vec::new();
        self.state.quadlets_error = None;
        self.state.quadlets_loading = true;
        self.refresh_quadlets(cx);
        cx.notify();
    }

    /// 关闭 Quadlet 弹窗；在途请求结果到达后会被丢弃。
    pub(crate) fn close_quadlets_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.quadlets_dialog_open = false;
        self.state.quadlets_request = None;
        self.state.quadlets_action_request = None;
        self.state.quadlets_loading = false;
        cx.notify();
    }

    /// 刷新 Quadlet 单元列表；已有在途请求时直接复用。
    pub(crate) fn refresh_quadlets(&mut self, cx: &mut Context<Self>) {
        if !self.state.quadlets_dialog_open || self.state.quadlets_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.quadlets_request = Some(request_id);
        self.state.quadlets_loading = true;
        self.state.quadlets_error = None;
        self.send_tracked(
            request_id,
            ContainerCommand::ListQuadlets {
                tab_id: self.tab_id,
                request_id,
            },
        );
        cx.notify();
    }

    /// 对 Quadlet 单元执行 systemd 操作，完成后自动刷新列表。
    pub(crate) fn quadlet_action(
        &mut self,
        unit: String,
        action: transom_core::ContainerQuadletAction,
        cx: &mut Context<Self>,
    ) {
        if self.state.quadlets_request.is_some() || self.state.quadlets_action_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.quadlets_action_request = Some(request_id);
        self.state.quadlets_loading = true;
        self.state.quadlets_error = None;
        self.send_tracked(
            request_id,
            ContainerCommand::QuadletAction {
                tab_id: self.tab_id,
                request_id,
                unit,
                action,
            },
        );
        cx.notify();
    }

    /// 应用 Quadlet 列表响应；请求已被替换或弹窗已关闭时丢弃结果。
    ///
    /// 返回是否真的写入了列表，供调用方决定是否触发重绘。
    pub(crate) fn apply_quadlets_listed(
        &mut self,
        request_id: ContainerRequestId,
        units: Vec<transom_core::ContainerQuadletUnit>,
    ) -> bool {
        if self.state.quadlets_request != Some(request_id) || !self.state.quadlets_dialog_open {
            return false;
        }
        self.state.quadlets = units;
        self.state.quadlets_request = None;
        self.state.quadlets_loading = false;
        self.state.quadlets_error = None;
        true
    }
}

impl AppShell {
    /// 处理 Quadlet 列表响应：按请求 id 找到所属标签页并写回实体。
    pub(super) fn on_container_quadlets_listed(
        &mut self,
        request_id: ContainerRequestId,
        units: Vec<transom_core::ContainerQuadletUnit>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
            return;
        };
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            let applied = workspace.update(cx, |workspace, _| {
                workspace.apply_quadlets_listed(request_id, units)
            });
            if applied {
                cx.notify();
            }
        }
    }
}
