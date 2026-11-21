//! 端口转发命令处理。

use super::super::Internal;
use super::super::state::{ControllerCtx, ControllerState};
use transom_core::{
    AppError, ConnectionId, ErrorCode, ForwardAppEvent, ForwardCommand, ForwardId, ForwardRule,
    ForwardState, ForwardStatus, TabId,
};
use transom_net::{start_forward, test_forward_target};

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
        ForwardCommand::ForwardProbe {
            tab_id,
            connection_id,
            rule_id,
            rule,
        } => {
            handle_forward_probe(state, ctx, tab_id, connection_id, rule_id, rule).await;
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
                        meter: None,
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
                    meter: None,
                },
            }
            .into(),
        )
        .await;
}

/// 目标端口连通性探测：与建连无关的真实 TCP 拨号，结果一次性回流。
///
/// 探测不需要 SSH 通道（目标就是本机可达性验证），但结果事件仍携带
/// 标签与连接归属，界面据此把结果落到对应规则卡片下。
async fn handle_forward_probe(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
    rule_id: ForwardId,
    rule: ForwardRule,
) {
    let event_tx = ctx.event_tx.clone();
    // 探测是短生命周期任务，纳入统一收尾集合；不等待完成，结果经事件回流。
    state.cleanup_set.spawn(async move {
        let result = test_forward_target(&rule).await;
        let _ = event_tx
            .send_async(
                ForwardAppEvent::ForwardProbeResult {
                    tab_id,
                    connection_id,
                    rule_id,
                    result,
                }
                .into(),
            )
            .await;
    });
}
