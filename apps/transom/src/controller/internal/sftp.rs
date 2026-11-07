//! SFTP 客户端就绪：归属校验、缓存写入与操作派发。

use crate::controller::Internal;
use crate::controller::sftp_exec;
use crate::controller::state::{ControllerCtx, ControllerState};
use transom_core::SftpAppEvent;

/// SFTP 客户端就绪：校验连接归属后写入缓存并派发操作。
pub(super) async fn handle_sftp_ready(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    let Internal::SftpReady {
        tab_id: requested_tab,
        connection_id,
        op_id,
        op,
        result,
    } = internal
    else {
        // 调用方按变体精确调用，其它形态直接忽略（不可达路径）。
        return;
    };
    // 提交前校验归属：只要该连接仍在活跃连接表中即合法。
    let valid_owner = state
        .session_by_connection(connection_id)
        .map(|(tab_id, _)| tab_id);
    let Some(owner_tab) = valid_owner else {
        drop(result);
        return;
    };
    if requested_tab != owner_tab {
        log::debug!(
            "SFTP 客户端按连接归属缓存，事件回报给请求窗格，请求窗格: {requested_tab}，归属窗格: {owner_tab}"
        );
    }
    // 打开期间可能已被取消或标签已关闭：按请求标签登记的任务不存在时丢弃
    // 已建立的客户端。取消命令携带的正是请求标签，键必须与之一致，
    // 归属标签只用来落地客户端缓存与远端句柄。
    if !state.sftp_tasks.contains_key(&(requested_tab, op_id)) {
        log::debug!("SFTP 操作在打开期间已被取消，丢弃已建立的客户端，窗格: {requested_tab}");
        drop(result);
        return;
    }
    state.sftp_tasks.remove(&(requested_tab, op_id));
    match result {
        Ok(client) => {
            if let Some(session) = state.sessions.get_mut(&owner_tab) {
                if op.is_transfer() {
                    session.transfer_sftp_client = Some(client.clone());
                } else {
                    session.sftp_client = Some(client.clone());
                }
            }
            let remote = state
                .sessions
                .get(&owner_tab)
                .and_then(|s| s.remote.clone());
            sftp_exec::spawn_sftp_op(
                &mut state.sftp_tasks,
                ctx.event_tx.clone(),
                sftp_exec::SpawnSftpOpParams {
                    tab_id: requested_tab,
                    connection_id,
                    op_id,
                    client,
                    remote,
                    op,
                },
            );
        }
        Err(error) => {
            // 打开专用传输通道失败时尝试回退至主控制通道，兼顾只支持单会话（MaxSessions=1）的受限服务器
            if op.is_transfer()
                && let Some(session) = state.sessions.get(&owner_tab)
                && let Some(fallback_client) = session.sftp_client.clone()
            {
                let fallback_remote = session.remote.clone();
                log::warn!(
                    "打开专用 SFTP 传输通道失败，回退至主控制通道，标签: {owner_tab}，原因: {error}"
                );
                sftp_exec::spawn_sftp_op(
                    &mut state.sftp_tasks,
                    ctx.event_tx.clone(),
                    sftp_exec::SpawnSftpOpParams {
                        tab_id: requested_tab,
                        connection_id,
                        op_id,
                        client: fallback_client,
                        remote: fallback_remote,
                        op,
                    },
                );
                return;
            }
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpFailed {
                        tab_id: requested_tab,
                        connection_id,
                        op_id,
                        error,
                    }
                    .into(),
                )
                .await;
        }
    }
}
