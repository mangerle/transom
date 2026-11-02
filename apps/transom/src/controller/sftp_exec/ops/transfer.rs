//! SFTP 传输类操作：单文件与目录树的上传、下载。
//!
//! 单文件传输的“传输 -> 校验”是一个闭环：校验发现不一致且本次是续传时，
//! 由本模块丢弃目标端内容从头覆盖重传一次，而不是把不一致当成最终结果交给
//! 用户手点重试。

use super::SftpExecutionContext;
use super::throttled_progress;
use crate::controller::sftp_finish::{
    VerifyContext, finish_sftp, finish_sftp_summary, verify_transfer_result,
};
use std::path::{Path, PathBuf};
use transom_core::{
    AppError, ErrorCode, SftpAppEvent, TransferFilter, TransferStart, VerifyOptions,
};
use transom_net::SftpClient;
use transom_net::sftp::TransferProgressResult;

pub(super) async fn handle_download(
    ctx: &SftpExecutionContext,
    remote_path: String,
    local_path: PathBuf,
    start: TransferStart,
    verify: VerifyOptions,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let transfer = FileTransferTask {
        context: ctx,
        direction: FileDirection::Download,
        local_path: &local_path,
        remote_path: &remote_path,
    };
    let result = transfer.run(start, verify, progress).await;
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
    start: TransferStart,
    verify: VerifyOptions,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let transfer = FileTransferTask {
        context: ctx,
        direction: FileDirection::Upload,
        local_path: &local_path,
        remote_path: &remote_path,
    };
    let result = transfer.run(start, verify, progress).await;
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
    filter: TransferFilter,
    verify: VerifyOptions,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .upload_dir(&local_path, &remote_path, &filter, verify, progress)
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
    filter: TransferFilter,
    verify: VerifyOptions,
) {
    let progress = throttled_progress(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
    );
    let result = ctx
        .client
        .download_dir(&remote_path, &local_path, &filter, verify, progress)
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

/// 单文件传输方向；目录任务不走这里，因此只保留两种合法方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileDirection {
    Upload,
    Download,
}

/// 一次单文件传输请求：连接上下文与两端路径。
struct FileTransferTask<'a> {
    context: &'a SftpExecutionContext,
    direction: FileDirection,
    local_path: &'a Path,
    remote_path: &'a str,
}

impl FileTransferTask<'_> {
    /// 执行“传输 + 校验”闭环并返回已传字节数与是否完成校验。
    ///
    /// 设计原理（Why）：续传的前提是目标端前缀确实来自本任务，而源文件可能在
    /// 中断期间被外部修改，此时前缀与源内容不再一致——只有传输后校验能发现。
    /// 发现后唯一正确的恢复是丢弃目标端内容从头覆盖，所以这里自动重传一次；
    /// 真正以覆盖方式写完仍然不一致时不再重复同样的写入，直接原样报错。
    ///
    /// 代价与局限：一次续传失败可能带来一整次额外全量传输；这是用带宽换
    /// 数据正确性，且只在“续传 + 校验开启”且确实不一致时发生。
    async fn run<F>(
        self,
        start: TransferStart,
        verify: VerifyOptions,
        mut progress: F,
    ) -> Result<(u64, bool), AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let mut start = start;
        loop {
            let result = self.run_once(start, &mut progress).await?;
            let verify_ctx = self.verify_context(verify);
            match verify_transfer_result(verify_ctx, Ok(result)).await {
                Ok(outcome) => return Ok(outcome),
                Err(error)
                    if error.code == ErrorCode::SftpVerifyFailed
                        && start != TransferStart::Overwrite =>
                {
                    log::warn!(
                        "续传结果校验不一致，丢弃目标端内容后从头覆盖重传，本地: {}，远端: {}，原因: {error}",
                        self.local_path.display(),
                        self.remote_path
                    );
                    let _ = self
                        .context
                        .event_tx
                        .send_async(
                            SftpAppEvent::SftpRetrying {
                                tab_id: self.context.tab_id,
                                connection_id: self.context.connection_id,
                                op_id: self.context.op_id,
                                reason: error.to_string(),
                            }
                            .into(),
                        )
                        .await;
                    start = TransferStart::Overwrite;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 按起点执行一次传输；节流器跨重试复用，两次尝试共用同一条进度通道。
    async fn run_once<F>(
        &self,
        start: TransferStart,
        progress: &mut F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let client: &SftpClient = &self.context.client;
        match self.direction {
            FileDirection::Upload => {
                client
                    .upload_resumable(self.local_path, self.remote_path, start, progress)
                    .await
            }
            FileDirection::Download => {
                client
                    .download_resumable(self.remote_path, self.local_path, start, progress)
                    .await
            }
        }
    }

    /// 组装校验上下文：事件通道与两端路径都来自本次传输，避免调用点重复拼装。
    fn verify_context(&self, options: VerifyOptions) -> VerifyContext<'_> {
        VerifyContext {
            event_tx: &self.context.event_tx,
            tab_id: self.context.tab_id,
            connection_id: self.context.connection_id,
            op_id: self.context.op_id,
            client: &self.context.client,
            local_path: self.local_path,
            remote_path: self.remote_path,
            options,
        }
    }
}
