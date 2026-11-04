//! 网络连接容器状态管理：打开、取消与提交。
//!
//! 设计原理（Why）：通过全局居中弹窗输入容器名称或 ID 快速将容器接入当前网络；
//! 操作提交后进入 busy 状态并自动追踪刷新网络与容器列表。

use super::*;
use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use transom_core::{ContainerCommand, ContainerRequestId, TabId};

impl AppShell {
    /// 打开网络连接容器弹窗。
    pub(crate) fn open_network_connect(
        &mut self,
        tab_id: TabId,
        network: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.network_connect.placeholder").to_string(),
            )
        });
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.connect_network = Some((network, input));
        }
        cx.notify();
    }

    /// 取消网络连接容器。
    pub(crate) fn cancel_network_connect(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.connect_network = None;
        }
        cx.notify();
    }

    /// 提交将容器连入当前网络。
    pub(crate) fn submit_network_connect(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((network, input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.connect_network.clone())
        else {
            return;
        };

        let raw = input.read(cx).text().to_string();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.show_toast(
                rust_i18n::t!("container.network_connect.invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        }

        let container = trimmed.to_string();
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.connect_network = None;
        }

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ConnectNetwork {
                tab_id,
                request_id,
                network,
                container,
            },
        );
        cx.notify();
    }
}
