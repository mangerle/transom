//! SFTP 命令调度分发与客户端缓存处理。

use super::ops::execute_sftp_op;
use super::remote_transfer::{execute_remote_transfer_op, spawn_remote_transfer_op};
use crate::controller::Internal;
use crate::controller::session_context::ManagedSessionContext;
use std::collections::HashMap;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, SftpAppEvent, SftpOperation, TabId, TransferId,
};
use transom_net::SftpClient;

/// SFTP 命令的调度参数：主循环状态表的借用与命令载荷。
pub(crate) struct SftpDispatch<'a> {
    pub(crate) sessions: &'a mut HashMap<TabId, ManagedSessionContext>,
    pub(crate) sftp_tasks: &'a mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    pub(crate) event_tx: flume::Sender<AppEvent>,
    pub(crate) internal_tx: flume::Sender<Internal>,
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) op_id: TransferId,
    pub(crate) op: SftpOperation,
}

/// SFTP 命令调度（即时返回）：校验连接归属与客户端缓存后派发执行；
/// 客户端未初始化时把打开动作交给任务、结果经 [`Internal::SftpReady`]
/// 回流提交，分发循环不为网络往返等待。
pub(crate) fn dispatch_sftp_op(dispatch: SftpDispatch<'_>) {
    let SftpDispatch {
        sessions,
        sftp_tasks,
        event_tx,
        internal_tx,
        tab_id,
        connection_id,
        op_id,
        op,
    } = dispatch;

    if let SftpOperation::RemoteTransfer {
        source_connection_id,
        source_path,
        target_connection_id,
        target_path,
        is_dir,
        filter,
        preserve_attributes,
    } = op
    {
        let source_remote = sessions
            .values()
            .find(|s| s.connection_id == source_connection_id)
            .and_then(|s| s.remote.clone());
        let target_remote = sessions
            .values()
            .find(|s| s.connection_id == target_connection_id)
            .and_then(|s| s.remote.clone());
        let (Some(source_remote), Some(target_remote)) = (source_remote, target_remote) else {
            send_sftp_not_ready(
                &event_tx,
                tab_id,
                connection_id,
                op_id,
                "源端或目标端 SSH 连接已断开",
            );
            return;
        };

        let cached_source = sessions
            .values()
            .find(|s| s.connection_id == source_connection_id)
            .and_then(|s| s.sftp_client.clone());
        let cached_target = sessions
            .values()
            .find(|s| s.connection_id == target_connection_id)
            .and_then(|s| s.sftp_client.clone());

        if let (Some(sc), Some(tc)) = (cached_source.clone(), cached_target.clone()) {
            spawn_remote_transfer_op(
                sftp_tasks,
                event_tx,
                tab_id,
                connection_id,
                op_id,
                sc,
                tc,
                source_path,
                target_path,
                is_dir,
                filter,
                preserve_attributes,
            );
        } else {
            let event_tx_clone = event_tx.clone();
            let handle = tokio::spawn(async move {
                let sc = match cached_source {
                    Some(c) => Ok(c),
                    None => SftpClient::open(&source_remote).await,
                };
                let tc = match cached_target {
                    Some(c) => Ok(c),
                    None => SftpClient::open(&target_remote).await,
                };
                match (sc, tc) {
                    (Ok(sc), Ok(tc)) => {
                        execute_remote_transfer_op(
                            event_tx_clone,
                            tab_id,
                            connection_id,
                            op_id,
                            sc,
                            tc,
                            source_path,
                            target_path,
                            is_dir,
                            filter,
                            preserve_attributes,
                        )
                        .await;
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        let _ = event_tx_clone
                            .send_async(
                                SftpAppEvent::SftpFailed {
                                    tab_id,
                                    connection_id,
                                    op_id,
                                    error: err,
                                }
                                .into(),
                            )
                            .await;
                    }
                }
            });
            sftp_tasks.retain(|_, h| !h.is_finished());
            sftp_tasks.insert((tab_id, op_id), handle);
        }
        return;
    }

    let (_target_tab_id, remote, cached) = if let Some(session) = sessions.get(&tab_id)
        && session.connection_id == connection_id
        && let Some(ref remote) = session.remote
    {
        (tab_id, remote.clone(), session.sftp_client.clone())
    } else if let Some((owner_tab, session)) = sessions
        .iter()
        .find(|(_, s)| s.connection_id == connection_id && s.remote.is_some())
    {
        (
            *owner_tab,
            session.remote.clone().unwrap(),
            session.sftp_client.clone(),
        )
    } else {
        send_sftp_not_ready(
            &event_tx,
            tab_id,
            connection_id,
            op_id,
            "SSH 连接尚未就绪或已断开",
        );
        return;
    };

    match cached {
        Some(client) => spawn_sftp_op(
            sftp_tasks,
            event_tx,
            tab_id,
            connection_id,
            op_id,
            client,
            op,
        ),
        None => {
            let remote = remote.clone();
            tokio::spawn(async move {
                let result = SftpClient::open(&remote).await;
                let _ = internal_tx
                    .send_async(Internal::SftpReady {
                        tab_id,
                        connection_id,
                        op_id,
                        op,
                        result,
                    })
                    .await;
            });
        }
    }
}

/// 客户端未就绪时向界面回失败事件的统一出口。
fn send_sftp_not_ready(
    event_tx: &flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    reason: &'static str,
) {
    let _ = event_tx.try_send(
        SftpAppEvent::SftpFailed {
            tab_id,
            connection_id,
            op_id,
            error: AppError::with_detail(ErrorCode::ChannelFailed, reason),
        }
        .into(),
    );
}

/// 派发单个 SFTP 操作执行：全部操作纳入任务表跟踪，标签关闭与停机时可统一取消。
pub(crate) fn spawn_sftp_op(
    sftp_tasks: &mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    client: SftpClient,
    op: SftpOperation,
) {
    let handle = tokio::spawn(execute_sftp_op(
        event_tx,
        tab_id,
        connection_id,
        op_id,
        client,
        op,
    ));
    sftp_tasks.retain(|_, h| !h.is_finished());
    sftp_tasks.insert((tab_id, op_id), handle);
}
