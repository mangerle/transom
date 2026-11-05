//! Quadlet 单元列表与 systemd 用户单元操作。
//!
//! Quadlet 是宿主侧文件，不存在引擎 API；所有文件扫描与 systemctl 调用都
//! 放进阻塞线程池，避免卡住 tokio worker。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::ListQuadlets { request_id, .. } => {
            let event_tx = ctx.event_tx.clone();
            state.container_ops.spawn(async move {
                match tokio::task::spawn_blocking(transom_container::quadlet::list).await {
                    Ok(Ok(units)) => {
                        send_event(
                            &event_tx,
                            ContainerAppEvent::QuadletsListed { request_id, units },
                        )
                        .await;
                    }
                    Ok(Err(error)) => send_failed(&event_tx, request_id, error).await,
                    Err(error) => {
                        send_event(
                            &event_tx,
                            ContainerAppEvent::Failed {
                                request_id,
                                error: AppError::with_detail(
                                    ErrorCode::ContainerApi,
                                    format!("Quadlet 扫描任务失败: {error}"),
                                ),
                            },
                        )
                        .await;
                    }
                }
            });
        }
        ContainerCommand::QuadletAction {
            request_id,
            unit,
            action,
            ..
        } => {
            let event_tx = ctx.event_tx.clone();
            state.container_ops.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    transom_container::quadlet::action(&unit, action)
                })
                .await;
                match result {
                    Ok(Ok(())) => {
                        send_event(
                            &event_tx,
                            ContainerAppEvent::ActionFinished {
                                request_id,
                                error: None,
                            },
                        )
                        .await;
                    }
                    Ok(Err(error)) => send_failed(&event_tx, request_id, error).await,
                    Err(error) => {
                        send_event(
                            &event_tx,
                            ContainerAppEvent::Failed {
                                request_id,
                                error: AppError::with_detail(
                                    ErrorCode::ContainerApi,
                                    format!("Quadlet 操作任务失败: {error}"),
                                ),
                            },
                        )
                        .await;
                    }
                }
            });
        }
        _ => {}
    }
}
