//! 容器内端口一键建立 SSH 本地端口转发交互逻辑。
//!
//! 设计原理（Why）：远程容器内部往往运行着未向公网直接开放的调试服务（如 Redis、MySQL、
//! 内部微服务控制台）。通过将容器端口与 Transom 底层的 SSH 端口转发引擎打通，
//! 开发者可在前端一键建立本地安全的 SSH 隧道，无需繁琐的手工命令行配置。
//!
//! 动作归属（Why）：取消、选择目标会话与提交校验只依赖本标签页状态，落在
//! [`ContainerWorkspace`] 上；启动转发要写外壳的转发状态与命令总线，因此提交时
//! 携带全部参数的 [`ContainerWorkspaceEvent::StartPortForward`] 交外壳执行。
//! 打开弹窗需要枚举全部 SSH 标签页，仍由外壳作为窗口级入口完成。

use super::*;
use crate::container::types::{ContainerForwardDialog, ContainerForwardOpenParams};
use gpui_kit::component::input::InputState;
use transom_core::{Connection, ConnectionId, TabId};

impl AppShell {
    /// 打开为指定容器内端口建立 SSH 本地端口转发的弹窗。
    ///
    /// 为什么留在外壳：目标候选来自全部标签页的活跃 SSH 连接，属于跨领域快照；
    /// 外壳把快照与输入控件一次性注入工作区实体，实体渲染时不反查外壳。
    pub(crate) fn open_container_port_forward(
        &mut self,
        tab_id: TabId,
        container_name: &str,
        private_port: u16,
        published_port: Option<u16>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 收集所有处于活跃连接态的 SSH 会话（排除本地会话）。
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
                .default_value(published_port.unwrap_or(private_port).to_string())
                .placeholder(rust_i18n::t!("container.forward.local_port_placeholder").to_string())
        });

        let target_host_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("127.0.0.1".to_string())
                .placeholder(rust_i18n::t!("container.forward.target_host_placeholder").to_string())
        });

        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        workspace.update(cx, |workspace, cx| {
            workspace.open_forward_dialog(
                ContainerForwardOpenParams {
                    container_name: container_name.to_string(),
                    private_port,
                    published_port,
                    local_port_input,
                    target_host_input,
                    ssh_sessions,
                },
                cx,
            );
        });
    }
}

impl ContainerWorkspace {
    /// 用外壳注入的候选会话与输入控件打开端口转发弹窗。
    fn open_forward_dialog(&mut self, params: ContainerForwardOpenParams, cx: &mut Context<Self>) {
        self.state.forward_dialog = Some(ContainerForwardDialog {
            container_name: params.container_name,
            private_port: params.private_port,
            published_port: params.published_port,
            local_port_input: params.local_port_input,
            target_host_input: params.target_host_input,
            ssh_sessions: params.ssh_sessions,
            selected_ssh_index: 0,
        });
        cx.notify();
    }

    /// 取消端口转发弹窗。
    pub(crate) fn cancel_container_port_forward(&mut self, cx: &mut Context<Self>) {
        self.state.forward_dialog = None;
        cx.notify();
    }

    /// 切换端口转发目标 SSH 会话。
    pub(crate) fn select_forward_ssh_session(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(dialog) = self.state.forward_dialog.as_mut() else {
            return;
        };
        if index >= dialog.ssh_sessions.len() {
            return;
        }
        dialog.selected_ssh_index = index;
        cx.notify();
    }

    /// 提交端口转发创建。
    ///
    /// 校验失败只写实体 notice（渲染帧统一转成通知），不关闭弹窗；成功后
    /// 关闭弹窗并发出启动事件，由外壳写入转发状态并投递命令。
    pub(crate) fn submit_container_port_forward(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.state.forward_dialog.as_ref() else {
            return;
        };
        let ssh_sessions = dialog.ssh_sessions.clone();
        let local_port_input = dialog.local_port_input.clone();
        let target_host_input = dialog.target_host_input.clone();
        let selected_index = dialog.selected_ssh_index;
        let container_name = dialog.container_name.clone();
        let private_port = dialog.private_port;
        let published_port = dialog.published_port;

        if ssh_sessions.is_empty() {
            self.state.notice =
                Some(rust_i18n::t!("container.forward.no_session_selected").to_string());
            cx.notify();
            return;
        }

        let local_port_str = local_port_input.read(cx).value().trim().to_string();
        let local_port: u16 = match local_port_str.parse() {
            Ok(port) if port > 0 => port,
            _ => {
                self.state.notice =
                    Some(rust_i18n::t!("container.forward.invalid_port").to_string());
                cx.notify();
                return;
            }
        };

        let target_host_str = target_host_input.read(cx).value().trim().to_string();
        let target_host = if target_host_str.is_empty() {
            "127.0.0.1".to_string()
        } else {
            target_host_str
        };

        let Some((_, ssh_tab_id, connection_id)) = ssh_sessions.get(selected_index).cloned() else {
            self.state.notice =
                Some(rust_i18n::t!("container.forward.no_session_selected").to_string());
            cx.notify();
            return;
        };

        let target_port = forward_target_port(private_port, published_port);
        let rule_name = format!("{container_name}:{private_port}");

        self.state.forward_dialog = None;
        self.state.notice = Some(
            rust_i18n::t!(
                "container.forward.success",
                container = container_name,
                local = local_port,
                remote = target_port
            )
            .to_string(),
        );
        cx.emit(ContainerWorkspaceEvent::StartPortForward {
            ssh_tab_id,
            connection_id,
            rule_name,
            bind_port: local_port,
            target_host,
            target_port,
        });
        cx.notify();
    }
}

/// 转发目标端口：优先使用宿主机实际发布的端口。
fn forward_target_port(private_port: u16, published_port: Option<u16>) -> u16 {
    published_port.unwrap_or(private_port)
}

#[cfg(test)]
mod tests {
    use super::forward_target_port;

    #[test]
    fn forwarding_targets_published_port_before_container_port() {
        assert_eq!(forward_target_port(80, Some(8080)), 8080);
        assert_eq!(forward_target_port(80, None), 80);
    }
}
