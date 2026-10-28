//! SFTP 目录对比与增量同步操作。

use super::SftpExecutionContext;
use super::throttled_progress;
use crate::controller::sftp_finish::finish_sftp_summary;
use std::path::PathBuf;
use transom_core::{SftpAppEvent, SyncItem, TransferFilter};
pub(super) async fn handle_compare_dirs(
    ctx: &SftpExecutionContext,
    local_path: PathBuf,
    remote_path: String,
    filter: TransferFilter,
) {
    match ctx
        .client
        .compare_directories(&local_path, &remote_path, &filter)
        .await
    {
        Ok(comparison) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpDirCompared {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        comparison,
                    }
                    .into(),
                )
                .await;
        }
        Err(error) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpFailed {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        error,
                    }
                    .into(),
                )
                .await;
        }
    }
}

pub(super) async fn handle_sync_dirs(
    ctx: &SftpExecutionContext,
    local_path: PathBuf,
    remote_path: String,
    items: Vec<SyncItem>,
    verify: bool,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .sync_items(&local_path, &remote_path, &items, verify, progress)
        .await;
    finish_sftp_summary(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}
