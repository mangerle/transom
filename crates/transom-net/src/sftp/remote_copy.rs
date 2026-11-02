//! SFTP 跨远端流式互传：客户端充当中转管道。
//!
//! 两端服务器通常没有网络直连或凭证互信，Transom 读取源端后经内存缓冲区写入
//! 目标端，全程不落本地磁盘；与单文件上传下载的起点语义无关，故独立成模块。

use super::client::{SftpClient, TRANSFER_CHUNK, sftp_error, sftp_io_error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use transom_core::AppError;

impl SftpClient {
    /// 跨远端流式传输单个文件：从当前 SFTP 会话读取并通过内存缓冲区写入目标 SFTP 会话。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：两台远端服务器之间通常没有网络直连或 SSH 凭证互信，Transom 客户端充当中转管道；
    /// - 核心优势：基于内存定长缓冲区的流式转发，零本地磁盘文件落盘与清理成本；
    ///   按选项同步源端文件的修改时间与 POSIX 权限；
    /// - 代价与局限：传输总耗时取决于两端带宽的短板。
    ///
    /// # Errors
    /// 读取源文件或写入目标文件失败时返回 `ErrorCode::SftpFailed`。
    pub async fn transfer_to_remote<F>(
        &self,
        source_path: &str,
        target_sftp: &SftpClient,
        target_path: &str,
        preserve_attributes: bool,
        mut progress: F,
    ) -> Result<u64, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let mut source = self.session.open(source_path).await.map_err(sftp_error)?;
        let source_meta = source.metadata().await.ok();
        let total = source_meta.as_ref().and_then(|m| m.size);
        let mtime = source_meta.as_ref().and_then(|m| m.mtime);
        let permissions = source_meta.as_ref().and_then(|m| m.permissions);

        let mut target = target_sftp
            .session
            .create(target_path)
            .await
            .map_err(sftp_error)?;

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut transferred = 0u64;
        progress(0, total);

        loop {
            let count = source.read(&mut buffer).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            target
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            transferred = transferred.saturating_add(count as u64);
            progress(transferred, total);
        }

        target.shutdown().await.map_err(sftp_io_error)?;
        let _ = source.close().await;

        if preserve_attributes {
            let mut attrs = russh_sftp::protocol::FileAttributes::empty();
            if let Some(m) = mtime {
                attrs.mtime = Some(m);
                attrs.atime = Some(m);
            }
            if let Some(p) = permissions {
                attrs.permissions = Some(p);
            }
            if !attrs.is_empty()
                && let Err(error) = target_sftp.session.set_metadata(target_path, attrs).await
            {
                log::debug!("跨远端保留元数据失败，路径: {target_path}，原因: {error}");
            }
        }

        Ok(transferred)
    }
}
