//! SFTP 查询类操作：目录列举与属性读取。

use super::SftpExecutionContext;
use transom_core::SftpAppEvent;
pub(super) async fn handle_list(ctx: &SftpExecutionContext, path: String) {
    match ctx.client.list(&path).await {
        Ok((path, entries)) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpListing {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        path,
                        entries,
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

pub(super) async fn handle_stat(ctx: &SftpExecutionContext, path: String) {
    match ctx.client.stat(&path).await {
        Ok(metadata) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpMetadata {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        metadata,
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
