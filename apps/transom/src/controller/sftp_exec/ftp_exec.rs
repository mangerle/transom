//! FTP 会话的文件面板操作执行器。
//!
//! 设计原理（Why）：面板只认 `SftpOperation` 契约与 `ConnectionId`，FTP 的
//! 能力面小于 SFTP——本执行器在同一个契约上实现子集，不支持的操作立即以
//! `ErrorCode::NotSupported` 回失败事件，走面板既有的错误横幅路径，UI 不需要
//! 感知协议差异；传输校验（远端哈希比对）在 FTP 下没有协议级等价物，一律
//! 按未校验收尾，`VerifyOptions` 被忽略而不是报错。
//!
//! 传输类操作与 SFTP 侧同构：任务执行体先打开独立传输连接（见
//! `FtpClient::open_transfer_client`），取消即整条废弃，交互连接不受影响。

use crate::controller::sftp_exec::progress::throttled_progress;
use crate::controller::sftp_finish::{finish_sftp, finish_sftp_none, finish_sftp_summary};
use std::collections::HashMap;
use std::path::PathBuf;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, RemoteDeleteTarget, SftpAppEvent, SftpOperation,
    TabId, TransferFilter, TransferId, TransferStart,
};
use transom_net::FtpClient;

/// FTP 任务执行上下文：与 `SftpExecutionContext` 对齐的参数收敛对象。
pub(crate) struct FtpExecutionContext {
    pub(crate) event_tx: flume::Sender<AppEvent>,
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) op_id: TransferId,
    pub(crate) client: FtpClient,
}

/// 派发单个 FTP 操作执行：与 SFTP 共用同一张任务表，标签关闭与停机时
/// 统一取消。
pub(crate) fn spawn_ftp_op(
    sftp_tasks: &mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    client: FtpClient,
    op: SftpOperation,
) {
    let handle = tokio::spawn(execute_ftp_op(
        FtpExecutionContext {
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

/// 执行单项 FTP 操作的主入口分发；不支持的操作即时回"不支持"失败事件。
pub(crate) async fn execute_ftp_op(ctx: FtpExecutionContext, op: SftpOperation) {
    match op {
        SftpOperation::List { path } => handle_list(&ctx, path).await,
        SftpOperation::Stat { path } => handle_stat(&ctx, path).await,
        SftpOperation::Mkdir { path } => {
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
        SftpOperation::Rename { from, to } => {
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
        SftpOperation::Delete { path, is_dir } => {
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
        SftpOperation::DeleteMany { targets } => handle_delete_many(&ctx, targets).await,
        SftpOperation::Chmod { path, permissions } => {
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
        SftpOperation::Download {
            remote_path,
            local_path,
            start,
            verify: _,
        } => handle_download(&ctx, remote_path, local_path, start).await,
        SftpOperation::Upload {
            local_path,
            remote_path,
            start,
            verify: _,
        } => handle_upload(&ctx, local_path, remote_path, start).await,
        SftpOperation::UploadDir {
            local_path,
            remote_path,
            filter,
            verify: _,
        } => handle_upload_dir(&ctx, local_path, remote_path, filter).await,
        SftpOperation::DownloadDir {
            remote_path,
            local_path,
            filter,
            verify: _,
        } => handle_download_dir(&ctx, remote_path, local_path, filter).await,
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
        SftpOperation::Symlink { .. }
        | SftpOperation::SudoWriteText { .. }
        | SftpOperation::ProbeSudo
        | SftpOperation::CompareDirs { .. }
        | SftpOperation::SyncDirs { .. }
        | SftpOperation::Archive { .. }
        | SftpOperation::Extract { .. }
        | SftpOperation::Search { .. }
        | SftpOperation::RemoteTransfer { .. } => {
            send_unsupported(&ctx.event_tx, &ctx.tab_id, &ctx.connection_id, &ctx.op_id);
        }
    }
}

/// FTP 无协议级等价物的操作统一出口：错误码 `NotSupported`，detail 带操作名。
fn send_unsupported(
    event_tx: &flume::Sender<AppEvent>,
    tab_id: &TabId,
    connection_id: &ConnectionId,
    op_id: &TransferId,
) {
    let _ = event_tx.try_send(
        SftpAppEvent::SftpFailed {
            tab_id: *tab_id,
            connection_id: *connection_id,
            op_id: *op_id,
            error: AppError::with_detail(
                ErrorCode::NotSupported,
                "FTP 协议不支持该操作（软链接、远端归档、远端检索、提权写入、目录对比与同步、跨远端互传）",
            ),
        }
        .into(),
    );
}

async fn handle_list(ctx: &FtpExecutionContext, path: String) {
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
        Err(error) => send_failed(ctx, error).await,
    }
}

async fn handle_stat(ctx: &FtpExecutionContext, path: String) {
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
        Err(error) => send_failed(ctx, error).await,
    }
}

async fn handle_delete_many(ctx: &FtpExecutionContext, targets: Vec<RemoteDeleteTarget>) {
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

async fn handle_download(
    ctx: &FtpExecutionContext,
    remote_path: String,
    local_path: PathBuf,
    start: TransferStart,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .download_resumable(&remote_path, &local_path, start, progress)
        .await
        .map(|progressed| (progressed.transferred, false));
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
    ctx: &FtpExecutionContext,
    local_path: PathBuf,
    remote_path: String,
    start: TransferStart,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .upload_resumable(&local_path, &remote_path, start, progress)
        .await
        .map(|progressed| (progressed.transferred, false));
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
    ctx: &FtpExecutionContext,
    local_path: PathBuf,
    remote_path: String,
    filter: TransferFilter,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .upload_dir(&local_path, &remote_path, &filter, progress)
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
    ctx: &FtpExecutionContext,
    remote_path: String,
    local_path: PathBuf,
    filter: TransferFilter,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .download_dir(&remote_path, &local_path, &filter, progress)
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

async fn handle_preview(
    ctx: &FtpExecutionContext,
    path: String,
    mode: transom_core::PreviewMode,
    max_bytes: u64,
) {
    match ctx.client.read_preview(&path, mode, max_bytes).await {
        Ok((bytes, truncated)) => {
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpPreview {
                        tab_id: ctx.tab_id,
                        connection_id: ctx.connection_id,
                        op_id: ctx.op_id,
                        bytes,
                        truncated,
                    }
                    .into(),
                )
                .await;
        }
        Err(error) => send_failed(ctx, error).await,
    }
}

async fn handle_read_text(ctx: &FtpExecutionContext, path: String, max_bytes: u64) {
    match ctx.client.read_text(&path, max_bytes).await {
        Ok((content, meta)) => {
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
        Err(error) => send_failed(ctx, error).await,
    }
}

async fn handle_write_text(
    ctx: &FtpExecutionContext,
    path: String,
    content: Vec<u8>,
    expected: Option<transom_core::RemoteTextMeta>,
    force: bool,
    backup: bool,
) {
    let result = ctx
        .client
        .write_text(&path, &content, expected, force, backup)
        .await;
    match result {
        Ok(meta) => {
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
        Err(error) => send_failed(ctx, error).await,
    }
}

/// 失败事件的统一出口。
async fn send_failed(ctx: &FtpExecutionContext, error: AppError) {
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
