//! 引擎连接、探测与断开。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::Connect { tab_id, profile } => connect(state, ctx, tab_id, profile),
        ContainerCommand::Disconnect { tab_id } => {
            if let Some(task) = state.container_connects.remove(&tab_id) {
                task.abort();
            }
            if let Some(mut workspace) = state.container_workspaces.remove(&tab_id) {
                for request_id in workspace.shutdown() {
                    ctx.container.finish(request_id);
                }
            }
            let _ = ctx
                .event_tx
                .try_send(ContainerAppEvent::Disconnected { tab_id }.into());
        }
        ContainerCommand::TestConnection {
            request_id,
            profile,
        } => {
            let service = Arc::clone(&ctx.container);
            let event_tx = ctx.event_tx.clone();
            state.container_ops.spawn(async move {
                let (info, error) = match service.connect(&profile).await {
                    Ok(engine) => (Some(Box::new(engine.info)), None),
                    Err(error) => (None, Some(AppError::from(error))),
                };
                send_event(
                    &event_tx,
                    ContainerAppEvent::TestFinished {
                        request_id,
                        info,
                        error,
                    },
                )
                .await;
            });
        }
        ContainerCommand::Discover { request_id } => {
            let event_tx = ctx.event_tx.clone();
            state.container_ops.spawn(async move {
                let (engines, failures) = discover_all().await;
                let profiles: Vec<ContainerEngineProfile> = engines
                    .iter()
                    .map(|engine| {
                        ContainerEngineProfile::new(
                            engine.info.engine.kind.to_string(),
                            engine.info.engine.clone(),
                        )
                    })
                    .collect();
                log::info!(
                    "容器引擎发现完成，可用 {} 个，不可用 {} 项",
                    profiles.len(),
                    failures.len()
                );
                send_event(
                    &event_tx,
                    ContainerAppEvent::EnginesDiscovered {
                        request_id,
                        engines: profiles,
                        failures,
                    },
                )
                .await;
            });
        }
        _ => {}
    }
}
