//! Podman Pod 列表与启停操作。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::ListPods { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.list_pods().await },
            |request_id, pods| ContainerAppEvent::PodsListed { request_id, pods },
        ),
        ContainerCommand::PodAction {
            tab_id,
            request_id,
            pod,
            action,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.pod_action(&pod, action).await
        }),
        _ => {}
    }
}
