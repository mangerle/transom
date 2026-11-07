//! 端口转发就绪：归属校验、任务登记与状态回报。

use crate::controller::Internal;
use crate::controller::state::{ControllerCtx, ControllerState};
use transom_core::{ForwardAppEvent, ForwardState, ForwardStatus};

/// 端口转发建立完成：校验连接归属后登记任务并向界面回报状态。
pub(super) async fn handle_forward_ready(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    let Internal::ForwardReady {
        tab_id,
        connection_id,
        rule_id,
        result,
    } = internal
    else {
        // 调用方按变体精确调用，其它形态直接忽略（不可达路径）。
        return;
    };
    let is_current = state
        .sessions
        .get(&tab_id)
        .is_some_and(|s| s.connection_id == connection_id && s.remote.is_some());
    let status = match (is_current, result) {
        (false, Ok(task)) => {
            // 连接已失效：立即停止刚建立的转发并释放。
            state.cleanup_set.spawn(async move {
                task.stop().await;
            });
            ForwardStatus {
                rule_id,
                state: ForwardState::Stopped,
                error: None,
            }
        }
        (false, Err(_)) => return,
        (true, Ok(task)) => {
            let bound_port = task.bound_port;
            if let Some(session) = state.sessions.get_mut(&tab_id) {
                session.forwards.insert(rule_id, task);
            }
            ForwardStatus {
                rule_id,
                state: ForwardState::Active { bound_port },
                error: None,
            }
        }
        (true, Err(error)) => ForwardStatus {
            rule_id,
            state: ForwardState::Failed,
            error: Some(error),
        },
    };
    let _ = ctx
        .event_tx
        .send_async(
            ForwardAppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status,
            }
            .into(),
        )
        .await;
}
