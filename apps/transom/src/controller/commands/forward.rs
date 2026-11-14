//! 端口转发命令处理。

use super::super::Internal;
use super::super::state::{ControllerCtx, ControllerState};
use transom_core::{
    AppError, ConnectionId, ErrorCode, ForwardAppEvent, ForwardCommand, ForwardId, ForwardRule,
    ForwardState, ForwardStatus, TabId,
};
use transom_net::start_forward;

pub(super) async fn handle_forward_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ForwardCommand,
) {
    match command {
        ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        } => {
            handle_forward_start(state, ctx, tab_id, connection_id, rule).await;
        }
        ForwardCommand::ForwardStop {
            tab_id,
            connection_id,
            rule_id,
        } => {
            handle_forward_stop(state, ctx, tab_id, connection_id, rule_id).await;
        }
    }
}


async fn handle_forward_start(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
    rule: ForwardRule,
) {
    let remote = state.current_remote(tab_id, connection_id);

    let Some(remote) = remote else {
        let _ = ctx
            .event_tx
            .send_async(
                ForwardAppEvent::ForwardChanged {
                    tab_id,
                    connection_id,
                    status: ForwardStatus {
                        rule_id: rule.id,
                        state: ForwardState::Failed,
                        error: Some(AppError::with_detail(
                            ErrorCode::ForwardFailed,
                            "SSH connection is not ready",
                        )),
                    },
                }
                .into(),
            )
            .await;
        return;
    };

    let internal_tx = ctx.internal_tx.clone();
    state.cleanup_set.spawn(async move {
        let result = start_forward(&remote, &rule).await;
        let _ = internal_tx
            .send_async(Internal::ForwardReady {
                tab_id,
                connection_id,
                rule_id: rule.id,
                result,
            })
            .await;
    });
}

/// 停止端口转发：停止动作纳入统一收尾集合，停机时限时等待。
async fn handle_forward_stop(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
    rule_id: ForwardId,
) {
    let Some(session) = state.sessions.get_mut(&tab_id) else {
        return;
    };
    if session.connection_id != connection_id {
        return;
    }
    if let Some(task) = session.forwards.remove(&rule_id) {
        state.cleanup_set.spawn(async move {
            task.stop().await;
        });
    }
    let _ = ctx
        .event_tx
        .send_async(
            ForwardAppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status: ForwardStatus {
                    rule_id,
                    state: ForwardState::Stopped,
                    error: None,
                },
            }
            .into(),
        )
        .await;
}
