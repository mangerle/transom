//! 容器工作区连接与长任务完成事件。

use crate::controller::Internal;
use crate::controller::container_ops::{ContainerWorkspace, finish_task};
use crate::controller::state::{ControllerCtx, ControllerState};
use transom_core::ContainerAppEvent;

/// 镜像拉取任务结束：只回收工作区里的 JoinHandle。
pub(super) fn handle_container_pull_finished(state: &mut ControllerState, internal: Internal) {
    let Internal::ContainerPullTaskFinished { tab_id, task_id } = internal else {
        return;
    };
    if let Some(workspace) = state.container_workspaces.get_mut(&tab_id) {
        workspace.remove_pull_task(&task_id);
    }
}

/// 引擎工作区连接完成：保存引擎实例并通知界面。
pub(super) async fn handle_container_connected(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    let Internal::ContainerConnected { tab_id, result } = internal else {
        return;
    };
    // 探测任务已经完成，句柄可以直接回收。
    if let Some(handle) = state.container_connects.remove(&tab_id) {
        handle.abort();
    }
    match result {
        Ok(engine) => {
            let info = engine.info.clone();
            let mut ws = ContainerWorkspace::new(engine);
            ws.start_events_listener(tab_id, ctx.event_tx.clone());
            if let Some(mut previous) = state.container_workspaces.insert(tab_id, ws) {
                previous.shutdown();
            }
            let _ = ctx
                .event_tx
                .send_async(
                    ContainerAppEvent::Connected {
                        tab_id,
                        info: Box::new(info),
                    }
                    .into(),
                )
                .await;
        }
        Err(error) => {
            let _ = ctx
                .event_tx
                .send_async(ContainerAppEvent::ConnectFailed { tab_id, error }.into())
                .await;
        }
    }
}

/// 容器长任务结束：回收可取消句柄与取消标志登记。
pub(super) fn handle_container_finished(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    let Internal::ContainerTaskFinished { tab_id, request_id } = internal else {
        return;
    };
    finish_task(state, ctx, tab_id, request_id);
}
