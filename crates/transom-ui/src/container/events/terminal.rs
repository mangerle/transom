//! 容器控制台就绪：在详情终端页签内创建终端视图。

use super::*;

pub(super) fn on_container_exec_ready(
    shell: &mut AppShell,
    tab_id: TabId,
    request_id: ContainerRequestId,
    io: TerminalIo,
    exit: tokio::sync::oneshot::Receiver<()>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(workspace) = shell.state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, _| {
            workspace.take_request(request_id);
        });
    }
    drop(exit);
    if shell.state.workspace.tab(tab_id).is_none() {
        return;
    }
    let (terminal, subscription) =
        super::super::terminal::create_container_terminal(shell, io, tab_id, window, cx);
    if let Some(workspace) = shell.state.container_workspace(tab_id) {
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
