//! SFTP 命令调度分发与客户端缓存处理。

use super::ops::{SftpExecutionContext, execute_sftp_op};
use super::remote_transfer::{
    RemoteTransferParams, execute_remote_transfer_op, spawn_remote_transfer_op,
};
use crate::controller::Internal;
use crate::controller::session_context::ManagedSessionContext;
use crate::controller::state::find_session_by_connection;
use std::collections::HashMap;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, SftpAppEvent, SftpOperation, TabId,
    TransferFilter, TransferId,
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

/// 单个 SFTP 操作在事件与任务表中的归属坐标。
#[derive(Clone, Copy)]
struct SftpOpScope {
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
}

/// 跨远端互传的派发输入：两端连接查询、任务表与传输参数。
struct RemoteTransferDispatch<'a> {
    sessions: &'a mut HashMap<TabId, ManagedSessionContext>,
    sftp_tasks: &'a mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    event_tx: &'a flume::Sender<AppEvent>,
    scope: SftpOpScope,
    source_connection_id: ConnectionId,
    source_path: String,
    target_connection_id: ConnectionId,
    target_path: String,
    is_dir: bool,
    filter: TransferFilter,
    preserve_attributes: bool,
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
        dispatch_remote_transfer(RemoteTransferDispatch {
            sessions,
            sftp_tasks,
            event_tx: &event_tx,
            scope: SftpOpScope {
                tab_id,
                connection_id,
                op_id,
            },
            source_connection_id,
            source_path,
            target_connection_id,
            target_path,
            is_dir,
            filter,
            preserve_attributes,
        });
        return;
    }

    let (_target_tab_id, remote, cached) = if let Some(session) = sessions.get(&tab_id)
        && session.connection_id == connection_id
        && let Some(ref remote) = session.remote
    {
        (tab_id, remote.clone(), session.sftp_client.clone())
    } else if let Some((owner_tab, remote, client)) =
        find_session_by_connection(sessions, connection_id).and_then(|(tab_id, session)| {
            session
                .remote
                .clone()
                .map(|remote| (tab_id, remote, session.sftp_client.clone()))
        })
    {
        (owner_tab, remote, client)
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

/// 跨远端互传：优先复用两端缓存客户端，未就绪时在任务内打开并直接执行，
/// 失败经事件总线回报。独立成函数，避免普通操作的分发路径被长分支淹没。
fn dispatch_remote_transfer(dispatch: RemoteTransferDispatch<'_>) {
    let RemoteTransferDispatch {
        sessions,
        sftp_tasks,
        event_tx,
        scope,
        source_connection_id,
        source_path,
        target_connection_id,
        target_path,
        is_dir,
        filter,
        preserve_attributes,
    } = dispatch;
    let SftpOpScope {
        tab_id,
        connection_id,
        op_id,
    } = scope;

    let (source_remote, cached_source) = find_session_by_connection(sessions, source_connection_id)
        .map(|(_, session)| (session.remote.clone(), session.sftp_client.clone()))
        .unwrap_or((None, None));
    let (target_remote, cached_target) = find_session_by_connection(sessions, target_connection_id)
        .map(|(_, session)| (session.remote.clone(), session.sftp_client.clone()))
        .unwrap_or((None, None));
    let (Some(source_remote), Some(target_remote)) = (source_remote, target_remote) else {
        send_sftp_not_ready(
            event_tx,
            tab_id,
            connection_id,
            op_id,
            "源端或目标端 SSH 连接已断开",
        );
        return;
    };

    if let (Some(source_client), Some(target_client)) =
        (cached_source.clone(), cached_target.clone())
    {
        spawn_remote_transfer_op(
            sftp_tasks,
            event_tx.clone(),
            RemoteTransferParams {
                tab_id,
                connection_id,
                op_id,
                source_client,
                target_client,
                source_path,
                target_path,
                is_dir,
                filter,
                preserve_attributes,
            },
        );
    } else {
        let event_tx_clone = event_tx.clone();
        let handle = tokio::spawn(async move {
            let source = match cached_source {
                Some(client) => Ok(client),
                None => SftpClient::open(&source_remote).await,
            };
            let target = match cached_target {
                Some(client) => Ok(client),
                None => SftpClient::open(&target_remote).await,
            };
            match (source, target) {
                (Ok(source_client), Ok(target_client)) => {
                    execute_remote_transfer_op(
                        event_tx_clone,
                        RemoteTransferParams {
                            tab_id,
                            connection_id,
                            op_id,
                            source_client,
                            target_client,
                            source_path,
                            target_path,
                            is_dir,
                            filter,
                            preserve_attributes,
                        },
                    )
                    .await;
                }
                (Err(error), _) | (_, Err(error)) => {
                    let _ = event_tx_clone
                        .send_async(
                            SftpAppEvent::SftpFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error,
                            }
                            .into(),
                        )
                        .await;
                }
            }
        });
        sftp_tasks.retain(|_, handle| !handle.is_finished());
        sftp_tasks.insert((tab_id, op_id), handle);
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
        SftpExecutionContext {
            event_tx,
            tab_id,
            connection_id,
            op_id,
            client,
        },
        op,
    ));
    sftp_tasks.retain(|_, handle| !handle.is_finished());
    sftp_tasks.insert((tab_id, op_id), handle);
}
