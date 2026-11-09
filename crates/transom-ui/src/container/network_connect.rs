//! 网络连接容器状态管理：打开、取消与提交。
//!
//! 设计原理（Why）：通过全局居中弹窗输入容器名称或 ID 快速将容器接入当前网络；
//! 操作提交后进入 busy 状态并自动追踪刷新网络与容器列表。
//!
//! 动作归属（Why）：打开、取消与提交只读写本标签页状态并投递命令，全部落在
//! [`ContainerWorkspace`] 上；输入为空时的一次性提示经事件交外壳弹出。

use super::*;
use gpui_kit::component::input::InputState;

impl ContainerWorkspace {
    /// 打开网络连接容器弹窗。
    pub(crate) fn open_network_connect(
        &mut self,
        network: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.network_connect.placeholder").to_string())
        });
        self.state.connect_network = Some((network, input));
        cx.notify();
    }

    /// 取消网络连接容器。
    pub(crate) fn cancel_network_connect(&mut self, cx: &mut Context<Self>) {
        self.state.connect_network = None;
        cx.notify();
    }

    /// 提交将容器连入当前网络。
    pub(crate) fn submit_network_connect(&mut self, cx: &mut Context<Self>) {
        let Some((network, input)) = self.state.connect_network.clone() else {
            return;
        };

        let raw = input.read(cx).text().to_string();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            cx.emit(ContainerWorkspaceEvent::ShowToast {
                message: rust_i18n::t!("container.network_connect.invalid").to_string(),
                success: false,
            });
            return;
        }

        let container = trimmed.to_string();
        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.connect_network = None;
        self.send_tracked(
            request_id,
            ContainerCommand::ConnectNetwork {
                tab_id: self.tab_id,
                request_id,
                network,
                container,
            },
        );
        cx.notify();
    }
}
