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

/// 提权写入的输入集合：路径、内容、指纹、强制标记、备份意图与口令必须成套出现。
pub(super) struct SudoWriteSpec {
    pub(super) path: String,
    pub(super) content: Vec<u8>,
    pub(super) expected: Option<RemoteTextMeta>,
    pub(super) force: bool,
    pub(super) backup: bool,
    pub(super) password: Option<String>,
}

pub(super) async fn handle_sudo_write_text(ctx: &SftpExecutionContext, spec: SudoWriteSpec) {
    let Some(ref remote) = ctx.remote else {
        emit_text_failed(
            ctx,
            AppError::with_detail(
                ErrorCode::SftpFailed,
                "当前连接缺少底层的 SSH 交互通道，无法发起 Sudo 提权写入",
            ),
        )
        .await;
        return;
    };

    // 提权通道绕过 SFTP 写接口，但并发指纹与 BOM/CRLF 还原必须与普通写回同一口径。
    let payload = match ctx
        .client
        .prepare_sudo_write(&spec.path, &spec.content, spec.expected, spec.force)
        .await
    {
        Ok(payload) => payload,
        Err(error) if error.code == ErrorCode::SftpTextConflict => {
            emit_text_conflict(ctx, error).await;
            return;
        }
        Err(error) => {
            emit_text_failed(ctx, error).await;
            return;
        }
    };

    let request = transom_net::SudoWriteRequest {
        target: &spec.path,
        payload: &payload,
        password: spec.password.as_deref(),
        backup: spec.backup,
    };
    match transom_net::sudo_write(remote, &request).await {
        Ok(report) => {
            // 写入成功后查询最新大小与修改时间同步给 UI 缓存；stat 失败时用提交
            // 字节数兜底，避免把一次成功写入显示成失败。
            let meta = match ctx.client.stat(&spec.path).await {
                Ok(stat) => RemoteTextMeta {
                    size: stat.size,
                    mtime: stat.mtime,
                    bom: spec.expected.is_some_and(|m| m.bom),
                    crlf: spec.expected.is_some_and(|m| m.crlf),
                    transcoded: false,
                },
                Err(_) => RemoteTextMeta {
                    size: report.bytes,
                    mtime: Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as u32,
                    ),
                    bom: spec.expected.is_some_and(|m| m.bom),
                    crlf: spec.expected.is_some_and(|m| m.crlf),
                    transcoded: false,
                },
            };
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
        Err(error) => emit_text_failed(ctx, error).await,
    }
}

/// 探测远端提权能力：结果决定界面是直接写、弹口令框还是说明不可用。
pub(super) async fn handle_probe_sudo(ctx: &SftpExecutionContext) {
    let Some(ref remote) = ctx.remote else {
        emit_text_failed(
            ctx,
            AppError::with_detail(
                ErrorCode::SftpFailed,
                "当前连接缺少底层的 SSH 交互通道，无法探测提权能力",
            ),
        )
        .await;
        return;
    };
    match transom_net::probe_sudo(remote).await {
        Ok(capability) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SudoProbed {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        capability,
                    }
                    .into(),
                )
                .await;
        }
        Err(error) => emit_text_failed(ctx, error).await,
    }
}

async fn emit_text_failed(ctx: &SftpExecutionContext, error: AppError) {
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

async fn emit_text_conflict(ctx: &SftpExecutionContext, error: AppError) {
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
