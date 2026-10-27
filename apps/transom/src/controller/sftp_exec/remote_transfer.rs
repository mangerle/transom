//! 跨远端中转流式传输执行。

use super::progress::throttled_progress;
use crate::controller::sftp_finish::{finish_sftp, finish_sftp_summary};
use std::collections::HashMap;
use transom_core::{AppEvent, ConnectionId, TabId, TransferFilter, TransferId};
use transom_net::SftpClient;

/// 派发跨远端流式传输任务。
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_remote_transfer_op(
    sftp_tasks: &mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    source_client: SftpClient,
    target_client: SftpClient,
    source_path: String,
    target_path: String,
    is_dir: bool,
    filter: TransferFilter,
    preserve_attributes: bool,
) {
    let handle = tokio::spawn(execute_remote_transfer_op(
        event_tx,
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
    ));
    sftp_tasks.retain(|_, h| !h.is_finished());
    sftp_tasks.insert((tab_id, op_id), handle);
}

/// 执行跨远端流式传输操作（文件单项或整棵目录树）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_remote_transfer_op(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    source_client: SftpClient,
    target_client: SftpClient,
    source_path: String,
    target_path: String,
    is_dir: bool,
    filter: TransferFilter,
    preserve_attributes: bool,
) {
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
