//! 目录并发传输单个文件的执行任务。

use super::SharedProgress;
use super::path::local_io_error;
use crate::sftp::{SftpClient, TransferProgressResult, notify_shared_progress};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use transom_core::AppError;

/// 目录传输单个文件的并发任务上下文。
///
/// 设计原理（Why）：
/// - 实现初衷：目录传输的回调是调用方的泛型闭包，若让它经"外层闭包包
///   async 块"多级嵌套再被 `tokio::spawn` 要求 `'static`，会触发 rustc
///   的 HRTT 推断限制（FnOnce is not general enough）；
/// - 核心优势：入口处把回调装箱为具体类型并连同参数收进本结构，任务体是
///   具名 async 函数，结构体内不再出现闭包嵌套闭包，完全绕开该限制；
/// - 代价与局限：一次堆装箱（整个目录传输仅一次），换取可编译且清晰的
///   任务边界。
pub(super) struct DirFileTask {
    pub(super) client: SftpClient,
    pub(super) local_path: PathBuf,
    pub(super) remote_path: String,
    pub(super) resume: bool,
    pub(super) verify: bool,
    pub(super) total: u64,
    pub(super) transferred: Arc<AtomicU64>,
    pub(super) progress: SharedProgress,
}

impl DirFileTask {
    /// 把传输层回调的累计字节换算成增量，聚合进共享进度。
    fn note_progress(&self, current: u64, last_reported: &mut u64) {
        let delta = current.saturating_sub(*last_reported);
        *last_reported = current;
        if delta > 0 {
            let done = self
                .transferred
                .fetch_add(delta, Ordering::Relaxed)
                .saturating_add(delta);
            notify_shared_progress(&self.progress, done, Some(self.total));
        }
    }

    /// 校验（可选）并补齐进度回调未覆盖的尾部字节。
    ///
    /// 传输层回调只到最近的块边界，剩余字节在这里补上，避免进度条停在 99%。
    async fn finish(
        &self,
        result: TransferProgressResult,
        last_reported: u64,
    ) -> Result<(), AppError> {
        if self.verify {
            self.client
                .verify_file_with_known_local(
                    &self.local_path,
                    &self.remote_path,
                    result.local_sha256.as_deref(),
                )
                .await?;
        }
        let tail = result.transferred.saturating_sub(last_reported);
        if tail > 0 {
            let done = self
                .transferred
                .fetch_add(tail, Ordering::Relaxed)
                .saturating_add(tail);
            notify_shared_progress(&self.progress, done, Some(self.total));
        }
        Ok(())
    }

    /// 上传单个文件：进度聚合后按需校验并补齐尾部。
    pub(super) async fn run_upload(self) -> Result<(), AppError> {
        let mut last_reported = 0u64;
        let result = self
            .client
            .upload_resumable(
                &self.local_path,
                &self.remote_path,
                self.resume,
                |current, _| self.note_progress(current, &mut last_reported),
            )
            .await?;
        self.finish(result, last_reported).await
    }

    /// 下载单个文件：确保本地父目录存在后拉取，进度聚合与上传同构。
    pub(super) async fn run_download(self) -> Result<(), AppError> {
        if let Some(parent) = self.local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| local_io_error("创建本地父目录失败", parent, error))?;
        }
        let mut last_reported = 0u64;
        let result = self
            .client
            .download_resumable(
                &self.remote_path,
                &self.local_path,
                self.resume,
                |current, _| self.note_progress(current, &mut last_reported),
            )
            .await?;
        self.finish(result, last_reported).await
    }
}
