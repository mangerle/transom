//! SFTP 单项基础操作执行（下载、上传、目录递归、属性、文本编辑与目录对比）。
//!
//! 设计原理（Why）：
//! - 将平铺的 6 个参数收敛为 [`SftpExecutionContext`] 上下文对象；
//! - 采用 Handler 模式将各操作分支独立为细粒度处理函数，主分发函数控制在 35 行以内；
//! - 各处理函数单一职责，便于独立测试、维护与添加超时保护。

use super::progress::throttled_progress;
use crate::controller::sftp_finish::{
    PreviewRequest, finish_sftp, finish_sftp_none, finish_sftp_summary, read_preview,
    verify_transfer_result,
};
use std::path::PathBuf;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, PreviewMode, RemoteDeleteTarget, RemoteTextMeta,
    SftpAppEvent, SftpOperation, SyncItem, TabId, TransferFilter, TransferId,
};
use transom_net::SftpClient;

/// 编辑器读取远端文本的最长等待时间。
const TEXT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// 编辑器回传文本的最长等待时间。写入需要远端落盘并回读确认，取 5 分钟。
const TEXT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// SFTP 任务执行上下文对象：收敛原平铺参数。
pub(crate) struct SftpExecutionContext {
    pub(crate) event_tx: flume::Sender<AppEvent>,
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) op_id: TransferId,
    pub(crate) client: SftpClient,
}

/// 执行单项 SFTP 操作的主入口分发。
pub(crate) async fn execute_sftp_op(ctx: SftpExecutionContext, op: SftpOperation) {
    match op {
        SftpOperation::List { path } => handle_list(&ctx, path).await,
        SftpOperation::Download {
            remote_path,
            local_path,
            resume,
            verify,
        } => handle_download(&ctx, remote_path, local_path, resume, verify).await,
        SftpOperation::Upload {
            local_path,
            remote_path,
            resume,
            verify,
        } => handle_upload(&ctx, local_path, remote_path, resume, verify).await,
        SftpOperation::UploadDir {
            local_path,
            remote_path,
            resume,
            filter,
            verify,
        } => handle_upload_dir(&ctx, local_path, remote_path, resume, filter, verify).await,
        SftpOperation::DownloadDir {
            remote_path,
            local_path,
            resume,
            filter,
            verify,
        } => handle_download_dir(&ctx, remote_path, local_path, resume, filter, verify).await,
        SftpOperation::Mkdir { path } => handle_mkdir(&ctx, path).await,
        SftpOperation::Rename { from, to } => handle_rename(&ctx, from, to).await,
        SftpOperation::Delete { path, is_dir } => handle_delete(&ctx, path, is_dir).await,
        SftpOperation::DeleteMany { targets } => handle_delete_many(&ctx, targets).await,
        SftpOperation::Chmod { path, permissions } => handle_chmod(&ctx, path, permissions).await,
        SftpOperation::Stat { path } => handle_stat(&ctx, path).await,
        SftpOperation::Preview {
            path,
            mode,
            max_bytes,
        } => handle_preview(&ctx, path, mode, max_bytes).await,
        SftpOperation::ReadText { path, max_bytes } => {
            handle_read_text(&ctx, path, max_bytes).await
        }
        SftpOperation::WriteText {
            path,
            content,
            expected,
            force,
            backup,
        } => handle_write_text(&ctx, path, content, expected, force, backup).await,
        SftpOperation::Symlink { target, link_path } => {
            handle_symlink(&ctx, target, link_path).await
        }
        SftpOperation::CompareDirs {
            local_path,
            remote_path,
            filter,
        } => handle_compare_dirs(&ctx, local_path, remote_path, filter).await,
        SftpOperation::SyncDirs {
            local_path,
            remote_path,
            items,
            verify,
        } => handle_sync_dirs(&ctx, local_path, remote_path, items, verify).await,
        SftpOperation::RemoteTransfer { .. } => {}
    }
}

async fn handle_list(ctx: &SftpExecutionContext, path: String) {
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

async fn handle_download(
    ctx: &SftpExecutionContext,
    remote_path: String,
    local_path: PathBuf,
    resume: bool,
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
        .download_resumable(&remote_path, &local_path, resume, progress)
        .await;
    let result =
        verify_transfer_result(&ctx.client, result, verify, &local_path, &remote_path).await;
    finish_sftp(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_upload(
    ctx: &SftpExecutionContext,
    local_path: PathBuf,
    remote_path: String,
    resume: bool,
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
        .upload_resumable(&local_path, &remote_path, resume, progress)
        .await;
    let result =
        verify_transfer_result(&ctx.client, result, verify, &local_path, &remote_path).await;
    finish_sftp(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_upload_dir(
    ctx: &SftpExecutionContext,
    local_path: PathBuf,
    remote_path: String,
    resume: bool,
    filter: TransferFilter,
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
        .upload_dir(&local_path, &remote_path, resume, &filter, verify, progress)
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

async fn handle_download_dir(
    ctx: &SftpExecutionContext,
    remote_path: String,
    local_path: PathBuf,
    resume: bool,
    filter: TransferFilter,
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
        .download_dir(&remote_path, &local_path, resume, &filter, verify, progress)
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

async fn handle_mkdir(ctx: &SftpExecutionContext, path: String) {
    let result = ctx.client.create_dir(&path).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_rename(ctx: &SftpExecutionContext, from: String, to: String) {
    let result = ctx.client.rename(&from, &to).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_delete(ctx: &SftpExecutionContext, path: String, is_dir: bool) {
    let result = ctx.client.delete(&path, is_dir).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_delete_many(ctx: &SftpExecutionContext, targets: Vec<RemoteDeleteTarget>) {
    let mut result = Ok(());
    for target in targets {
        if let Err(error) = ctx.client.delete(&target.path, target.is_dir).await {
            result = Err(error);
            break;
        }
    }
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_chmod(ctx: &SftpExecutionContext, path: String, permissions: u32) {
    let result = ctx.client.chmod(&path, permissions).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_stat(ctx: &SftpExecutionContext, path: String) {
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

async fn handle_preview(
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

async fn handle_read_text(ctx: &SftpExecutionContext, path: String, max_bytes: u64) {
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

async fn handle_write_text(
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

async fn handle_symlink(ctx: &SftpExecutionContext, target: String, link_path: String) {
    let result = ctx.client.symlink(&target, &link_path).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn handle_compare_dirs(
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

async fn handle_sync_dirs(
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
