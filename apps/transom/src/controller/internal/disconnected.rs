//! 断开通知：仅当连接归属匹配时清理附属任务与远端句柄。

use crate::controller::lifecycle::drop_local_host;
use crate::controller::state::{ControllerCtx, ControllerState};
use transom_core::{ConnectionId, TabId, TerminalAppEvent};

/// 断开通知：仅当前任连接生效时才清理全部附属任务与远端句柄。
pub(super) async fn handle_disconnected(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
) {
    let is_current = state.current_connection(tab_id) == Some(connection_id);
    if !is_current {
        return;
    }
    state.terminate_session(tab_id, "connection lost");
    drop_local_host(&state.local_host_tabs, tab_id);
    state.sftp_tasks.retain(|(t, _), handle| {
        if *t == tab_id {
            handle.abort();
            false
        } else {
            true
        }
    });
    let _ = ctx
        .event_tx
        .send_async(
            TerminalAppEvent::Disconnected {
                tab_id,
                connection_id,
            }
            .into(),
        )
        .await;
}
