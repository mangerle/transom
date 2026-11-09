//! Podman Pod 列表弹窗与启停操作。
//!
//! 动作归属（Why）：打开、关闭、刷新与启停只读写本标签页状态并投递命令，
//! 全部落在 [`ContainerWorkspace`] 上；外壳只负责把窗口级「打开弹窗」意图
//! 转成一次实体更新，不再持有这些动作。

use super::*;

impl ContainerWorkspace {
    /// 打开 Pod 弹窗并拉取列表；仅 Podman CLI 连接可用。
    pub(crate) fn open_pods_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.pods_dialog_open = true;
        self.state.pods = Vec::new();
        self.state.pods_error = None;
        self.state.pods_loading = true;
        self.refresh_pods(cx);
        cx.notify();
    }

    /// 关闭 Pod 弹窗；在途列表请求结果到达后会被丢弃。
    pub(crate) fn close_pods_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.pods_dialog_open = false;
        self.state.pods_request = None;
        self.state.pods_action_request = None;
        self.state.pods_loading = false;
        cx.notify();
    }

    /// 刷新 Pod 列表；已有在途请求时直接复用。
    pub(crate) fn refresh_pods(&mut self, cx: &mut Context<Self>) {
        if !self.state.pods_dialog_open || self.state.pods_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.pods_request = Some(request_id);
        self.state.pods_loading = true;
        self.state.pods_error = None;
        self.send_tracked(
            request_id,
            ContainerCommand::ListPods {
                tab_id: self.tab_id,
                request_id,
            },
        );
        cx.notify();
    }

    /// 对 Pod 执行启停/重启/删除，完成后自动刷新列表。
    pub(crate) fn pod_action(
        &mut self,
        pod: String,
        action: transom_core::ContainerPodAction,
        cx: &mut Context<Self>,
    ) {
        if self.state.pods_request.is_some() || self.state.pods_action_request.is_some() {
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.pods_action_request = Some(request_id);
        self.state.pods_loading = true;
        self.state.pods_error = None;
        self.send_tracked(
            request_id,
            ContainerCommand::PodAction {
                tab_id: self.tab_id,
                request_id,
                pod,
                action,
            },
        );
        cx.notify();
    }

    /// 应用 Pod 列表响应；请求已被替换或弹窗已关闭时丢弃结果。
    ///
    /// 返回是否真的写入了列表，供调用方决定是否触发重绘。
    pub(crate) fn apply_pods_listed(
        &mut self,
        request_id: ContainerRequestId,
        pods: Vec<transom_core::ContainerPodSummary>,
    ) -> bool {
        if self.state.pods_request != Some(request_id) || !self.state.pods_dialog_open {
            return false;
        }
        self.state.pods = pods;
        self.state.pods_request = None;
        self.state.pods_loading = false;
        self.state.pods_error = None;
        true
    }
}

impl AppShell {
    /// 处理 Pod 列表响应：按请求 id 找到所属标签页并写回实体。
    ///
    /// 设计原理（Why）：请求登记在实体内部，事件只带 `request_id`；外壳取走
    /// 登记后调用实体具名方法，不在闭包里拼装状态修改逻辑。
    pub(super) fn on_container_pods_listed(
        &mut self,
        request_id: ContainerRequestId,
        pods: Vec<transom_core::ContainerPodSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
            return;
        };
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            let applied = workspace.update(cx, |workspace, _| {
                workspace.apply_pods_listed(request_id, pods)
            });
            if applied {
                cx.notify();
            }
        }
    }
}
