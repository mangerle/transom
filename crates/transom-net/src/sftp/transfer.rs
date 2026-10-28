//! SFTP 单文件上传、下载、断点续传、跨远端流转与轻量预览。

use super::client::{SftpClient, TRANSFER_CHUNK, sftp_error, sftp_io_error};
use super::hash::hex_encode;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, PreviewMode};

/// 单文件传输执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferProgressResult {
    /// 实际传输并完成落盘或上传的字节数。
    pub transferred: u64,
    /// 在途计算的本地文件完整 SHA-256 哈希；若发生断点续传（非从零开始传输），则为 None。
    pub local_sha256: Option<String>,
}

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

    /// 下载远端文件到本地，覆盖已有内容。
    pub async fn download<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        progress: F,
    ) -> Result<u64, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        Ok(self
            .download_resumable(remote_path, local_path, false, progress)
            .await?
            .transferred)
    }

    /// 顺序下载远端文件，可按本地已有字节数续传。
    ///
    /// 设计原理（Why）：
    /// - 单句柄顺序读写，并发全部交给 russh-sftp 的深预取（见 `sftp_config`）
    ///   与提高后的 SSH 通道窗口。句柄数量不再影响吞吐，却决定了失败后的文件
    ///   形态：顺序写入保证本地文件始终是远端内容的连续前缀，本地大小就是
    ///   可信的断点；多句柄并发写各自偏移会在中途失败后留下空洞，续传时
    ///   按文件大小追加会跳过空洞，拼出损坏文件。
    /// - 与远端等大时直接视为完成，避免重复传输；本地比远端大说明目标残留了
    ///   多余尾部，必须截断重传。
    /// - 传输过程中同步计算在途 SHA-256 哈希，传输落盘完毕即可直接获取本地哈希，
    ///   彻底消除传输后的本地磁盘二次全量读取。
    /// - 代价与局限：单句柄没有多路并行，只能依赖库的预取深度；预取深度与
    ///   通道窗口的匹配关系集中维护在 `sftp_config`。
    ///
    /// # Errors
    /// 打开远端或本地文件、读取、写入失败时返回 `ErrorCode::SftpFailed`。
    /// 保留修改时间失败只记录调试日志，不影响传输结果。
    pub async fn download_resumable<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        resume: bool,
        mut progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let mut remote = self.session.open(remote_path).await.map_err(sftp_error)?;
        let remote_metadata = remote.metadata().await.ok();
        let total = remote_metadata.as_ref().and_then(|metadata| metadata.size);
        let mtime = remote_metadata.as_ref().and_then(|metadata| metadata.mtime);

        let mut offset = 0u64;
        let mut open_options = tokio::fs::OpenOptions::new();
        open_options.write(true).create(true);

        if resume && local_path.exists() {
            if let Ok(meta) = tokio::fs::metadata(local_path).await {
                let local_size = meta.len();
                match total {
                    Some(total_size) if local_size == total_size => {
                        if let Some(mtime) = mtime {
                            let _ = set_local_mtime(local_path, mtime).await;
                        }
                        progress(total_size, Some(total_size));
                        return Ok(TransferProgressResult {
                            transferred: total_size,
                            local_sha256: None,
                        });
                    }
                    Some(total_size) if local_size > total_size => {
                        // 目标比源大时不可能只是“尚未传完”，多出的尾部必须截断重传
                        open_options.truncate(true);
                    }
                    _ => {
                        offset = local_size;
                        open_options.append(true);
                    }
                }
            } else {
                open_options.truncate(true);
            }
        } else {
            open_options.truncate(true);
        }

        if offset > 0 {
            remote
                .seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(sftp_io_error)?;
        }

        let mut local = open_options.open(local_path).await.map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("打开本地文件失败 {}: {error}", local_path.display()),
            )
        })?;

        let mut local_hasher = if offset == 0 {
            Some(Sha256::new())
        } else {
            None
        };

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut transferred = offset;
        progress(transferred, total);

        loop {
            let count = remote.read(&mut buffer).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            if let Some(hasher) = local_hasher.as_mut() {
                hasher.update(&buffer[..count]);
            }
            local
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            transferred = transferred.saturating_add(count as u64);
            progress(transferred, total);
        }
        local.flush().await.map_err(sftp_io_error)?;
        let _ = remote.close().await;
        if let Some(mtime) = mtime
            && let Err(error) = set_local_mtime(local_path, mtime).await
        {
            log::debug!(
                "保留本地文件修改时间失败，路径: {}，原因: {error}",
                local_path.display()
            );
        }
        let local_sha256 = local_hasher.map(|h| hex_encode(&h.finalize()));
        Ok(TransferProgressResult {
            transferred,
            local_sha256,
        })
    }

    /// 上传本地文件到远端，覆盖已有内容。
    pub async fn upload<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress: F,
    ) -> Result<u64, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        Ok(self
            .upload_resumable(local_path, remote_path, false, progress)
            .await?
            .transferred)
    }

    /// 顺序上传本地文件，可按远端已有字节数续传。
    ///
    /// 设计原理（Why）：
    /// - 单句柄顺序写入，并发交给 `max_concurrent_writes` 深预取（见
    ///   `sftp_config`）。远端文件在失败后始终是本地内容的连续前缀，
    ///   `remote_size` 才是可信的续传断点；多句柄各写区间会在失败后留下
    ///   空洞，续传按远端大小追加会拼出损坏文件。
    /// - 全新上传用 `create` 一次性截断，不再需要并发块时代的“先 create
    ///   再各块 WRITE|CREATE”特判。
    /// - 两端等大时直接视为完成；远端比源大说明残留下多余尾部，截断重传。
    /// - 传输过程中同步计算本地读取流的 SHA-256 哈希，完全消除传输后的本地磁盘二次读盘。
    ///
    /// # Errors
    /// 打开远端或本地文件、读取、写入失败时返回 `ErrorCode::SftpFailed`。
    /// 保留远端修改时间失败只记录调试日志，不影响传输结果。
    pub async fn upload_resumable<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        resume: bool,
        mut progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let mut local = tokio::fs::File::open(local_path).await.map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("打开本地文件失败 {}: {error}", local_path.display()),
            )
        })?;
        let local_metadata = local.metadata().await.ok();
        let total = local_metadata.as_ref().map(|metadata| metadata.len());
        let local_mtime =
            unix_seconds(local_metadata.and_then(|metadata| metadata.modified().ok()));

        let mut offset = 0u64;
        let mut remote_file = if resume {
            if let Ok(meta) = self.session.metadata(remote_path).await {
                let remote_size = meta.size.unwrap_or(0);
                match total {
                    Some(total_size) if remote_size == total_size => {
                        if let Some(mtime) = local_mtime {
                            let _ = self.set_mtime(remote_path, mtime).await;
                        }
                        progress(total_size, Some(total_size));
                        return Ok(TransferProgressResult {
                            transferred: total_size,
                            local_sha256: None,
                        });
                    }
                    Some(total_size) if remote_size > total_size => {
                        // 远端比源文件大时不能视为已完成，截断后从头上传
                        self.session.create(remote_path).await.map_err(sftp_error)?
                    }
                    _ => {
                        offset = remote_size;
                        let mut f = self
                            .session
                            .open_with_flags(
                                remote_path,
                                russh_sftp::protocol::OpenFlags::WRITE
                                    | russh_sftp::protocol::OpenFlags::CREATE,
                            )
                            .await
                            .map_err(sftp_error)?;
                        if offset > 0 {
                            f.seek(std::io::SeekFrom::Start(offset))
                                .await
                                .map_err(sftp_io_error)?;
                            local
                                .seek(std::io::SeekFrom::Start(offset))
                                .await
                                .map_err(sftp_io_error)?;
                        }
                        f
                    }
                }
            } else {
                self.session.create(remote_path).await.map_err(sftp_error)?
            }
        } else {
            self.session.create(remote_path).await.map_err(sftp_error)?
        };

        let mut local_hasher = if offset == 0 {
            Some(Sha256::new())
        } else {
            None
        };

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut transferred = offset;
        progress(transferred, total);

        loop {
            let count = local.read(&mut buffer).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            if let Some(hasher) = local_hasher.as_mut() {
                hasher.update(&buffer[..count]);
            }
            remote_file
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            transferred = transferred.saturating_add(count as u64);
            progress(transferred, total);
        }
        remote_file.shutdown().await.map_err(sftp_io_error)?;
        if let Some(mtime) = local_mtime
            && let Err(error) = self.set_mtime(remote_path, mtime).await
        {
            log::debug!("保留远端文件修改时间失败，路径: {remote_path}，原因: {error}");
        }
        let local_sha256 = local_hasher.map(|h| hex_encode(&h.finalize()));
        Ok(TransferProgressResult {
            transferred,
            local_sha256,
        })
    }

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

/// 将本地文件修改时间设置为 Unix 秒；阻塞 IO 调度到专用线程池。
pub(crate) async fn set_local_mtime(path: &Path, mtime: u32) -> Result<(), AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path).map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("打开本地文件失败 {}: {error}", path.display()),
            )
        })?;
        file.set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(mtime)))
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("设置本地文件修改时间失败 {}: {error}", path.display()),
                )
            })
    })
    .await
    .map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("设置本地文件修改时间任务异常结束: {error}"),
        )
    })?
}

fn unix_seconds(time: Option<SystemTime>) -> Option<u32> {
    let seconds = time?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.min(u64::from(u32::MAX)) as u32)
}
