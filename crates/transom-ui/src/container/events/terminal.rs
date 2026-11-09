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
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.take_request(request_id);
            });
        }
        drop(exit);
        if self.state.workspace.tab(tab_id).is_none() {
            return;
        }
        let (terminal, subscription) = self.create_container_terminal(io, tab_id, window, cx);
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                let (user, shell) = workspace
                    .state
                    .terminal_request
                    .take()
                    .map(|(_, user, shell)| (user, shell))
                    .unwrap_or((None, None));
                workspace.state.terminal = Some(ContainerTerminalState {
                    view: terminal,
                    user,
                    shell,
                    _subscription: subscription,
                });
                workspace.state.detail_tab = ContainerDetailTab::Terminal;
                workspace.state.notice = None;
            });
        }
    }
}
