//! 容器控制台就绪：在详情终端页签内创建终端视图。

use super::*;

impl AppShell {
    pub(super) fn on_container_exec_ready(
        &mut self,
        tab_id: TabId,
        request_id: ContainerRequestId,
        io: TerminalIo,
        exit: tokio::sync::oneshot::Receiver<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.containers.requests.remove(&request_id);
        drop(exit);
        if self.state.workspace.tab(tab_id).is_none() {
            return;
        }
        let (terminal, subscription) = self.create_container_terminal(io, tab_id, window, cx);
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            let (user, shell) = workspace
                .terminal_request
                .take()
                .map(|(_, user, shell)| (user, shell))
                .unwrap_or((None, None));
            workspace.terminal = Some(ContainerTerminalState {
                view: terminal,
                user,
                shell,
                _subscription: subscription,
            });
            workspace.detail_tab = ContainerDetailTab::Terminal;
            workspace.notice = None;
        }
    }
}
