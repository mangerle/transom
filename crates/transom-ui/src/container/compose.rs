//! 编排项目的起停动作。

use super::*;

impl AppShell {
    /// 编排项目命令：`up` 直接下发，`down` 会删容器与网络，先过确认条。
    pub(crate) fn run_compose(
        &mut self,
        tab_id: TabId,
        name: String,
        action: transom_core::ComposeAction,
        cx: &mut Context<Self>,
    ) {
        if action.is_dangerous() {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.confirm = Some(ContainerConfirm::Compose(name));
            }
            cx.notify();
            return;
        }
        let Some(project) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| {
                workspace
                    .compose
                    .iter()
                    .find(|item| item.name == name)
                    .cloned()
            })
        else {
            return;
        };
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.notice = Some(
                rust_i18n::t!("container.compose.running", action = action.label()).to_string(),
            );
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::RunCompose {
                tab_id,
                request_id,
                project: Box::new(project),
                action,
            },
        );
        cx.notify();
    }
}
