//! SFTP 传输类操作：单文件与目录树的上传、下载。

use super::SftpExecutionContext;
use super::throttled_progress;
use crate::controller::sftp_finish::{finish_sftp, finish_sftp_summary, verify_transfer_result};
use std::path::PathBuf;
use transom_core::TransferFilter;
pub(super) async fn handle_download(
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

pub(super) async fn handle_upload(
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

pub(super) async fn handle_upload_dir(
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

pub(super) async fn handle_download_dir(
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
