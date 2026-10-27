//! 跨远端中转流式传输执行。

use super::progress::throttled_progress;
use crate::controller::sftp_finish::{finish_sftp, finish_sftp_summary};
use std::collections::HashMap;
use transom_core::{AppEvent, ConnectionId, TabId, TransferFilter, TransferId};
use transom_net::SftpClient;

/// 跨远端流式传输任务配置参数。
pub(crate) struct RemoteTransferParams {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) op_id: TransferId,
    pub(crate) source_client: SftpClient,
    pub(crate) target_client: SftpClient,
    pub(crate) source_path: String,
    pub(crate) target_path: String,
    pub(crate) is_dir: bool,
    pub(crate) filter: TransferFilter,
    pub(crate) preserve_attributes: bool,
}

/// 派发跨远端流式传输任务。
pub(crate) fn spawn_remote_transfer_op(
    sftp_tasks: &mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    event_tx: flume::Sender<AppEvent>,
    params: RemoteTransferParams,
) {
    let tab_id = params.tab_id;
    let op_id = params.op_id;
    let handle = tokio::spawn(execute_remote_transfer_op(event_tx, params));
    sftp_tasks.retain(|_, h| !h.is_finished());
    sftp_tasks.insert((tab_id, op_id), handle);
}

/// 执行跨远端流式传输操作（文件单项或整棵目录树）。
pub(crate) async fn execute_remote_transfer_op(
    event_tx: flume::Sender<AppEvent>,
    params: RemoteTransferParams,
) {
    let RemoteTransferParams {
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
    } = params;

    if is_dir {
        let result = source_client
            .transfer_dir_to_remote(
                &source_path,
                &target_client,
                &target_path,
                &filter,
                preserve_attributes,
                throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
            )
            .await;
        finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
    } else {
        let result = source_client
            .transfer_to_remote(
                &source_path,
                &target_client,
                &target_path,
                preserve_attributes,
                throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
            )
            .await;
        finish_sftp(
            event_tx,
            tab_id,
            connection_id,
            op_id,
            result.map(|bytes| (bytes, false)),
        )
        .await;
    }
}
