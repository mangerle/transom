//! SFTP 传输类操作：单文件与目录树的上传、下载。

use super::SftpExecutionContext;
use super::throttled_progress;
use crate::controller::sftp_finish::{
    VerifyContext, finish_sftp, finish_sftp_summary, verify_transfer_result,
};
use std::path::PathBuf;
use transom_core::{ChecksumAlgorithm, TransferFilter, VerifyMode};

pub(super) async fn handle_download(
    ctx: &SftpExecutionContext,
    remote_path: String,
    local_path: PathBuf,
    resume: bool,
    verify: bool,
    mode: VerifyMode,
    algorithm: ChecksumAlgorithm,
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
    let verify_ctx = VerifyContext {
        event_tx: &ctx.event_tx,
        tab_id: ctx.tab_id,
        connection_id: ctx.connection_id,
        op_id: ctx.op_id,
        client: &ctx.client,
        local_path: &local_path,
        remote_path: &remote_path,
        verify,
        mode,
        algorithm,
    };
    let result = verify_transfer_result(verify_ctx, result).await;
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
    mode: VerifyMode,
    algorithm: ChecksumAlgorithm,
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
    let verify_ctx = VerifyContext {
        event_tx: &ctx.event_tx,
        tab_id: ctx.tab_id,
        connection_id: ctx.connection_id,
        op_id: ctx.op_id,
        client: &ctx.client,
        local_path: &local_path,
        remote_path: &remote_path,
        verify,
        mode,
        algorithm,
    };
    let result = verify_transfer_result(verify_ctx, result).await;
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
