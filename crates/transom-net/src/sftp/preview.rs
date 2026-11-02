//! SFTP 悬停轻量预览读取。
//!
//! 预览是独立只读通道：不进入传输队列、不参与并发调度、不提供进度回调，
//! 与单文件传输的关注点不同，因此独立成模块。

use super::client::{SftpClient, TRANSFER_CHUNK, sftp_error, sftp_io_error};
use tokio::io::AsyncReadExt;
use transom_core::{AppError, ErrorCode, PreviewMode};

impl SftpClient {
    /// 读取远端文件内容用于悬停轻量预览。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：文本预览只需要文件头部若干字节即可立即出结果，
    ///   图片预览必须完整读取后交给渲染层解码，两种读取边界差异明显；
    /// - 核心优势：`Head` 模式只多读 1 字节判断截断，超大日志不会因预览被
    ///   整份读入内存；`Full` 模式先用服务端 size 提前拒绝超限文件，
    ///   元数据缺失时仍以“多读 1 字节”兜底；
    /// - 代价与局限：预览是独立只读通道，不会进入传输队列，也不提供进度回调；
    ///   因此只适合小体积内容，超限判断依赖调用方给出的上限。
    ///
    /// # Errors
    /// 路径不存在、权限不足、会话中断，或完整读取模式下文件超过 `max_bytes`
    /// 时返回 `ErrorCode::SftpFailed`。
    pub async fn read_preview(
        &self,
        path: &str,
        mode: PreviewMode,
        max_bytes: u64,
    ) -> Result<(Vec<u8>, bool), AppError> {
        let mut remote = self.session.open(path).await.map_err(sftp_error)?;
        let total = remote
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.size);
        if mode == PreviewMode::Full
            && let Some(total) = total
            && total > max_bytes
        {
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("文件 {path} 大小 {total} 字节，超过预览上限 {max_bytes} 字节"),
            ));
        }

        let limit = max_bytes.saturating_add(1);
        let capacity = total
            .map(|total| total.min(max_bytes))
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or(0);
        let mut buffer = Vec::with_capacity(capacity);
        let mut chunk = vec![0u8; TRANSFER_CHUNK];
        let mut read_total = 0u64;
        while read_total < limit {
            let remaining = usize::try_from(limit - read_total).unwrap_or(usize::MAX);
            let count = remote
                .read(&mut chunk[..remaining.min(TRANSFER_CHUNK)])
                .await
                .map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..count]);
            read_total = read_total.saturating_add(count as u64);
        }
        let _ = remote.close().await;

        let truncated = read_total > max_bytes;
        if truncated {
            if mode == PreviewMode::Full {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("文件 {path} 超过预览上限 {max_bytes} 字节"),
                ));
            }
            buffer.truncate(usize::try_from(max_bytes).unwrap_or(usize::MAX));
        }
        Ok((buffer, truncated))
    }
}
