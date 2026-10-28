//! SFTP 文本与预览类操作：预览、读取与写回远端文件。

use super::SftpExecutionContext;
use crate::controller::sftp_finish::{PreviewRequest, read_preview};
use transom_core::{AppError, ErrorCode, PreviewMode, RemoteTextMeta, SftpAppEvent};

/// 编辑器读取远端文本的最长等待时间。
const TEXT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// 编辑器回传文本的最长等待时间。写入需要远端落盘并回读确认，取 5 分钟。
const TEXT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
pub(super) async fn handle_preview(
    ctx: &SftpExecutionContext,
    path: String,
    mode: PreviewMode,
    max_bytes: u64,
) {
    read_preview(PreviewRequest {
        client: &ctx.client,
        event_tx: ctx.event_tx.clone(),
        tab_id: ctx.tab_id,
        connection_id: ctx.connection_id,
        op_id: ctx.op_id,
        path,
        mode,
        max_bytes,
    })
    .await;
}

pub(super) async fn handle_read_text(ctx: &SftpExecutionContext, path: String, max_bytes: u64) {
    let result =
        tokio::time::timeout(TEXT_READ_TIMEOUT, ctx.client.read_text(&path, max_bytes)).await;
    match result {
        Ok(Ok((content, meta))) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextRead {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        content,
                        meta,
                    }
                    .into(),
                )
                .await;
        }
        Ok(Err(error)) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextFailed {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        error,
                    }
                    .into(),
                )
                .await;
        }
        Err(_) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextFailed {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        error: AppError::with_detail(
                            ErrorCode::SftpFailed,
                            format!("读取远端文本超时（{} 秒）", TEXT_READ_TIMEOUT.as_secs()),
                        ),
                    }
                    .into(),
                )
                .await;
        }
    }
}

pub(super) async fn handle_write_text(
    ctx: &SftpExecutionContext,
    path: String,
    content: Vec<u8>,
    expected: Option<RemoteTextMeta>,
    force: bool,
    backup: bool,
) {
    let result = tokio::time::timeout(
        TEXT_WRITE_TIMEOUT,
        ctx.client
            .write_text(&path, &content, expected, force, backup),
    )
    .await;
    match result {
        Ok(Ok(meta)) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextWritten {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        meta,
                    }
                    .into(),
                )
                .await;
        }
        Ok(Err(error)) if error.code == ErrorCode::SftpTextConflict => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextConflict {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        error,
                    }
                    .into(),
                )
                .await;
        }
        Ok(Err(error)) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextFailed {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        error,
                    }
                    .into(),
                )
                .await;
        }
        Err(_) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpTextFailed {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        error: AppError::with_detail(
                            ErrorCode::SftpFailed,
                            format!("回传远端文本超时（{} 秒）", TEXT_WRITE_TIMEOUT.as_secs()),
                        ),
                    }
                    .into(),
                )
                .await;
        }
    }
}
