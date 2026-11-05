//! 容器内端口一键建立 SSH 本地端口转发交互逻辑。
//!
//! 设计原理（Why）：远程容器内部往往运行着未向公网直接开放的调试服务（如 Redis、MySQL、
//! 内部微服务控制台）。通过将容器端口与 Transom 底层的 SSH 端口转发引擎打通，
//! 开发者可在前端一键建立本地安全的 SSH 隧道，无需繁琐的手工命令行配置。

use crate::app::AppShell;
use crate::container::types::ContainerForwardDialog;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{
    BindAddress, Connection, ConnectionId, ForwardCommand, ForwardKind, ForwardRule, ForwardState,
    TabId, TargetAddress,
};

impl AppShell {
    /// 打开为指定容器内端口建立 SSH 本地端口转发的弹窗。
    pub(crate) fn open_container_port_forward(
        &mut self,
        tab_id: TabId,
        container_name: &str,
        remote_port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 收集所有处于活跃连接态的 SSH 会话（排除本地会话）
        let ssh_sessions: Vec<(String, TabId, ConnectionId)> = self
            .state
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| {
                let connection_id = tab.connection_id?;
                if matches!(&tab.session.connection, Connection::Local(_)) {
                    return None;
                }
                let name = if tab.session.name.trim().is_empty() {
                    tab.title.clone()
                } else {
                    tab.session.name.clone()
                };
                Some((name, tab.id, connection_id))
            })
            .collect();

        let local_port_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(remote_port.to_string())
                .placeholder(rust_i18n::t!("container.forward.local_port_placeholder").to_string())
        });

        let target_host_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("127.0.0.1".to_string())
                .placeholder(rust_i18n::t!("container.forward.target_host_placeholder").to_string())
        });

        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };

        workspace.forward_dialog = Some(ContainerForwardDialog {
            container_name: container_name.to_string(),
            remote_port,
            local_port_input,
            target_host_input,
            ssh_sessions,
            selected_ssh_index: 0,
        });

        cx.notify();
    }

    /// 取消端口转发弹窗。
    pub(crate) fn cancel_container_port_forward(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.forward_dialog = None;
            cx.notify();
        }
    }

    /// 切换端口转发目标 SSH 会话。
    pub(crate) fn select_forward_ssh_session(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id)
            && let Some(dialog) = workspace.forward_dialog.as_mut()
            && index < dialog.ssh_sessions.len()
        {
            dialog.selected_ssh_index = index;
            cx.notify();
        }
    }

    /// 提交端口转发创建。
    pub(crate) fn submit_container_port_forward(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(dialog) = workspace.forward_dialog.as_ref() else {
            return;
        };

        if dialog.ssh_sessions.is_empty() {
            workspace.notice =
                Some(rust_i18n::t!("container.forward.no_session_selected").to_string());
            cx.notify();
            return;
        }

        let local_port_str = dialog.local_port_input.read(cx).value().trim().to_string();
        let local_port: u16 = match local_port_str.parse() {
            Ok(port) if port > 0 => port,
            _ => {
                workspace.notice =
                    Some(rust_i18n::t!("container.forward.invalid_port").to_string());
                cx.notify();
                return;
            }
        };

        let target_host_str = dialog.target_host_input.read(cx).value().trim().to_string();
        let target_host = if target_host_str.is_empty() {
            "127.0.0.1".to_string()
        } else {
            target_host_str
        };

        let selected_index = dialog.selected_ssh_index;
        let Some((_, ssh_tab_id, connection_id)) = dialog.ssh_sessions.get(selected_index).cloned()
        else {
            workspace.notice =
                Some(rust_i18n::t!("container.forward.no_session_selected").to_string());
            cx.notify();
            return;
        };

        let container_name = dialog.container_name.clone();
        let remote_port = dialog.remote_port;

        let rule = ForwardRule::new(
            format!("{container_name}:{remote_port}"),
            ForwardKind::Local {
                bind: BindAddress {
                    host: "127.0.0.1".into(),
                    port: local_port,
                },
                target: TargetAddress {
                    host: target_host,
                    port: remote_port,
                },
            },
        );

        self.state
            .set_forward_status(ssh_tab_id, rule.id, ForwardState::Starting, None);
        self.state.send_command(ForwardCommand::ForwardStart {
            tab_id: ssh_tab_id,
            connection_id,
            rule,
        });

        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.forward_dialog = None;
            workspace.notice = Some(
                rust_i18n::t!(
                    "container.forward.success",
                    container = container_name,
                    local = local_port,
                    remote = remote_port
                )
                .to_string(),
            );
        }
        cx.notify();
    }
}
