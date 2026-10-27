//! SFTP 命令调度分发与客户端缓存处理。

use super::ops::execute_sftp_op;
use super::remote_transfer::{execute_remote_transfer_op, spawn_remote_transfer_op};
use crate::controller::Internal;
use std::collections::HashMap;
use transom_core::{AppError, AppEvent, ConnectionId, ErrorCode, SftpOperation, TabId, TransferId};
use transom_net::{SftpClient, SshRemote};

/// SFTP 命令的调度参数：主循环状态表的借用与命令载荷。
pub(crate) struct SftpDispatch<'a> {
    pub(crate) remotes: &'a HashMap<TabId, (ConnectionId, SshRemote)>,
    pub(crate) sftp_clients: &'a mut HashMap<TabId, SftpClient>,
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
        remotes,
        sftp_clients,
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
        let source_remote = remotes
            .values()
            .find(|(c, _)| *c == source_connection_id)
            .map(|(_, r)| r.clone());
        let target_remote = remotes
            .values()
            .find(|(c, _)| *c == target_connection_id)
            .map(|(_, r)| r.clone());
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

        let source_tab = remotes
            .iter()
            .find(|(_, (c, _))| *c == source_connection_id)
            .map(|(t, _)| *t);
        let target_tab = remotes
            .iter()
            .find(|(_, (c, _))| *c == target_connection_id)
            .map(|(t, _)| *t);

        let cached_source = source_tab.and_then(|t| sftp_clients.get(&t).cloned());
        let cached_target = target_tab.and_then(|t| sftp_clients.get(&t).cloned());

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
                            .send_async(AppEvent::SftpFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error: err,
                            })
                            .await;
                    }
                }
            });
            sftp_tasks.retain(|_, h| !h.is_finished());
            sftp_tasks.insert((tab_id, op_id), handle);
        }
        return;
    }

    let (target_tab_id, remote) = if let Some((current_connection, remote)) = remotes.get(&tab_id)
        && *current_connection == connection_id
    {
        (tab_id, remote.clone())
    } else if let Some((owner_tab, (_, remote))) =
        remotes.iter().find(|(_, (c, _))| *c == connection_id)
    {
        (*owner_tab, remote.clone())
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

    let cached = sftp_clients.get(&target_tab_id).cloned();
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
    let _ = event_tx.try_send(AppEvent::SftpFailed {
        tab_id,
        connection_id,
        op_id,
        error: AppError::with_detail(ErrorCode::ChannelFailed, reason),
    });
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
