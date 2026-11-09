//! SFTP 单文件上传、下载、断点续传、跨远端流转与轻量预览。
//!
//! 传输起点（覆盖或续传）一律由调用方显式声明，本层不再根据目标端已有尺寸
//! 反推断点：目标端“恰好存在同名文件”与“上次中断写了一半”是两件完全不同
//! 的事，靠尺寸猜测会把旧文件当成本次残留，同尺寸直接跳过、更短则把源文件
//! 尾部拼到旧内容之后，最终只能由传输后校验发现脏数据。

use super::client::{SftpClient, TRANSFER_CHUNK, sftp_error, sftp_io_error};
use super::dir::local_io_error;
use super::hash::hex_encode;
use russh_sftp::client::fs::File as RemoteFileHandle;
use russh_sftp::protocol::FileAttributes;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, TransferStart};

/// 单文件传输执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferProgressResult {
    /// 实际传输并完成落盘或上传的字节数。
    pub transferred: u64,
    /// 在途计算的本地文件完整 SHA-256 哈希；若发生断点续传（非从零开始传输），则为 None。
    pub local_sha256: Option<String>,
}

impl SftpClient {
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
            .download_resumable(remote_path, local_path, TransferStart::Overwrite, progress)
            .await?
            .transferred)
    }

    /// 顺序下载远端文件，按调用方声明的起点续传。
    ///
    /// 设计原理（Why）：
    /// - 单句柄顺序读写，并发全部交给 russh-sftp 的深预取（见 `sftp_config`）
    ///   与提高后的 SSH 通道窗口。句柄数量不再影响吞吐，却决定了失败后的文件
    ///   形态：顺序写入保证本地文件始终是远端内容的连续前缀，本地大小就是
    ///   可信的断点；多句柄并发写各自偏移会在中途失败后留下空洞，续传时
    ///   按文件大小追加会跳过空洞，拼出损坏文件。
    /// - 起点由调用方声明，本层不再用“本地已有字节数”猜测断点：本地存在同名
    ///   文件只说明它是要被覆盖的旧版本，不是上次中断的证据。`Overwrite` 一律
    ///   截断重写，只有 `Continue` 才从指定偏移追加，且偏移必须落在远端文件
    ///   长度之内，否则说明源已变短、断点失效，退回从头覆盖。
    /// - 续传结束后按实际写入字节数收尾截断，本地残留的旧尾部不会被保留。
    /// - 传输过程中同步计算在途 SHA-256 哈希（仅从 0 开始写时可用），传输落盘
    ///   完毕即可直接获取本地哈希，消除传输后的本地磁盘二次全量读取。
    ///
    /// # Errors
    /// 打开远端或本地文件、读取、写入失败时返回 `ErrorCode::SftpFailed`。
    /// 保留修改时间失败只记录调试日志，不影响传输结果。
    pub async fn download_resumable<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        start: TransferStart,
        mut progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let mut remote = self.session.open(remote_path).await.map_err(sftp_error)?;
        let remote_metadata = remote.metadata().await.ok();
        let total = remote_metadata.as_ref().and_then(|metadata| metadata.size);
        let mtime = remote_metadata.as_ref().and_then(|metadata| metadata.mtime);

        let (mut local, offset) = open_local_download_target(local_path, start, total).await?;
        if offset > 0 {
            remote
                .seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(sftp_io_error)?;
        }

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
        finish_download_target(local_path, &mut local, transferred, mtime).await?;
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
            .upload_resumable(local_path, remote_path, TransferStart::Overwrite, progress)
            .await?
            .transferred)
    }

    /// 顺序上传本地文件，按调用方声明的起点续传。
    ///
    /// 设计原理（Why）：
    /// - 单句柄顺序写入，并发交给 `max_concurrent_writes` 深预取（见
    ///   `sftp_config`）。远端文件在失败后始终是本地内容的连续前缀，
    ///   本任务记录的偏移才是可信的续传断点；多句柄各写区间会在失败后留下
    ///   空洞，续传按远端大小追加会拼出损坏文件。
    /// - 起点由调用方声明：远端存在同名文件只说明它是要被覆盖的旧版本，
    ///   不是上次中断的残留。`Overwrite` 直接 `create` 截断，只有 `Continue`
    ///   才从偏移处续写，且远端当前长度必须覆盖该偏移，否则退回从头覆盖。
    /// - 续传写完后若远端原有长度超过本次写入长度，按实际字节数收尾截断，
    ///   不把旧尾部留在文件末尾。
    /// - 传输过程中同步计算本地读取流的 SHA-256 哈希（仅从 0 开始写时可用），
    ///   完全消除传输后的本地磁盘二次读盘。
    ///
    /// # Errors
    /// 打开远端或本地文件、读取、写入失败时返回 `ErrorCode::SftpFailed`。
    /// 保留远端修改时间失败只记录调试日志，不影响传输结果。
    pub async fn upload_resumable<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        start: TransferStart,
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
        let source_len = total.unwrap_or(0);

        let UploadTarget {
            file: mut remote_file,
            offset,
            existing_len,
        } = self
            .open_remote_upload_target(remote_path, start, source_len)
            .await?;
        if offset > 0 {
            local
                .seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(sftp_io_error)?;
        }

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
        self.finish_upload_target(remote_path, transferred, existing_len, local_mtime)
            .await?;
        let local_sha256 = local_hasher.map(|h| hex_encode(&h.finalize()));
        Ok(TransferProgressResult {
            transferred,
            local_sha256,
        })
    }

    /// 上传收尾：截断续传残留的旧尾部，并尽力保留源文件修改时间。
    ///
    /// 设计原理（Why）：续传只覆盖 `offset` 之后的内容，远端原有长度若超过
    /// 本次写入长度，文件末尾会残留上一版数据；这种“尾巴”不会出现在字节流
    /// 里，只能显式按实际写入长度截断，否则落盘结果比源文件更长。
    ///
    /// # Errors
    /// 远端截断失败时返回 `ErrorCode::SftpFailed`；修改时间失败只记日志，
    /// 不影响传输结果的正确性。
    async fn finish_upload_target(
        &self,
        remote_path: &str,
        transferred: u64,
        previous_len: u64,
        mtime: Option<u32>,
    ) -> Result<(), AppError> {
        if previous_len > transferred {
            let mut attributes = FileAttributes::empty();
            attributes.size = Some(transferred);
            self.session
                .set_metadata(remote_path, attributes)
                .await
                .map_err(|error| {
                    AppError::with_detail(
                        ErrorCode::SftpFailed,
                        format!("截断远端文件尾部失败 {remote_path}: {error}"),
                    )
                })?;
        }
        if let Some(mtime) = mtime
            && let Err(error) = self.set_mtime(remote_path, mtime).await
        {
            log::debug!("保留远端文件修改时间失败，路径: {remote_path}，原因: {error}");
        }
        Ok(())
    }

    /// 按传输起点打开远端写入句柄。
    ///
    /// 设计原理（Why）：续传的合法性只能由调用方给出的偏移与被覆盖文件的
    /// 当前长度共同保证——远端若已被删除或截短到偏移以下，前缀不再完整，
    /// 继续追加会写出错位内容，此时必须退回从头覆盖，而不是报错或照样追加。
    ///
    /// 返回句柄、起始偏移以及打开前的远端长度：后者用于在收尾时判断是否
    /// 残留了旧尾部需要截断（覆盖模式恒为 0）。
    async fn open_remote_upload_target(
        &self,
        remote_path: &str,
        start: TransferStart,
        source_len: u64,
    ) -> Result<UploadTarget, AppError> {
        // 偏移超出源文件长度说明源已被截短，断点失效，必须从头覆盖；
        // 把偏移夹到源尾部只会写零字节并留下远端旧内容。
        let requested = match start {
            TransferStart::Continue { offset } if offset <= source_len && offset > 0 => offset,
            _ => 0,
        };
        if requested == 0 {
            return Ok(UploadTarget {
                file: self.session.create(remote_path).await.map_err(sftp_error)?,
                offset: 0,
                existing_len: 0,
            });
        }
        let existing_len = self
            .session
            .metadata(remote_path)
            .await
            .ok()
            .and_then(|metadata| metadata.size);
        let Some(existing_len) = existing_len.filter(|len| *len >= requested) else {
            log::debug!("远端文件不足以支撑续传偏移，退回从头覆盖，路径: {remote_path}");
            return Ok(UploadTarget {
                file: self.session.create(remote_path).await.map_err(sftp_error)?,
                offset: 0,
                existing_len: 0,
            });
        };
        let mut file: RemoteFileHandle = self
            .session
            .open_with_flags(
                remote_path,
                russh_sftp::protocol::OpenFlags::WRITE | russh_sftp::protocol::OpenFlags::CREATE,
            )
            .await
            .map_err(sftp_error)?;
        file.seek(std::io::SeekFrom::Start(requested))
            .await
            .map_err(sftp_io_error)?;
        Ok(UploadTarget {
            file,
            offset: requested,
            existing_len,
        })
    }
}

/// 上传目标的续传现场。
///
/// `existing_len` 是打开句柄前的远端长度：覆盖模式下无意义（恒为 0），
/// 续传时用于判断写入完成后是否残留了需要截断的旧尾部。
struct UploadTarget {
    /// 远端写入句柄。
    file: RemoteFileHandle,
    /// 实际起始偏移；0 表示从头覆盖。
    offset: u64,
    /// 打开前的远端文件长度。
    existing_len: u64,
}

/// 下载收尾：按实际写入字节数截断本地残留尾部，并尽力保留远端修改时间。
///
/// 覆盖模式打开时已经截断，这里的截断等价于取当前长度；续传模式下本地可能
/// 比源更长，多出的旧尾部必须显式裁掉，否则落盘结果比源文件更长。
async fn finish_download_target(
    local_path: &Path,
    local: &mut tokio::fs::File,
    transferred: u64,
    mtime: Option<u32>,
) -> Result<(), AppError> {
    local
        .set_len(transferred)
        .await
        .map_err(|error| local_io_error("截断本地文件尾部失败", local_path, error))?;
    if let Some(mtime) = mtime
        && let Err(error) = set_local_mtime(local_path, mtime).await
    {
        log::debug!(
            "保留本地文件修改时间失败，路径: {}，原因: {error}",
            local_path.display()
        );
    }
    Ok(())
}

/// 按传输起点打开本地下载目标，返回句柄与实际起始偏移。
///
/// 设计原理（Why）：续传成立的前提是本地已有的内容确实是远端内容的连续前缀，
/// 因此偏移必须落在远端文件长度之内，且本地文件不能短于该偏移；任一条件不
/// 满足都说明本地是无关的旧文件（或被外部清理过），只能截断重写。覆盖模式
/// 则不检查本地内容——本地存在什么与本次传输无关。
async fn open_local_download_target(
    local_path: &Path,
    start: TransferStart,
    remote_total: Option<u64>,
) -> Result<(tokio::fs::File, u64), AppError> {
    let requested = match (start, remote_total) {
        // 偏移超出远端长度说明源已变短，断点不再成立；夹到远端尾部只会
        // 写零字节并把本地旧内容当成新内容保留下来。
        (TransferStart::Continue { offset }, Some(total)) if offset <= total && offset > 0 => {
            offset
        }
        _ => 0,
    };
    let offset = if requested == 0 {
        0
    } else {
        match tokio::fs::metadata(local_path).await {
            Ok(metadata) if metadata.len() >= requested => requested,
            _ => {
                log::debug!(
                    "本地文件不足以支撑续传偏移，退回从头覆盖，路径: {}",
                    local_path.display()
                );
                0
            }
        }
    };
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true);
    if offset == 0 {
        options.truncate(true);
    }
    let mut file = options
        .open(local_path)
        .await
        .map_err(|error| local_io_error("打开本地文件失败", local_path, error))?;
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(sftp_io_error)?;
    }
    Ok((file, offset))
}

/// 将本地文件修改时间设置为 Unix 秒；阻塞 IO 调度到专用线程池。
pub(crate) async fn set_local_mtime(path: &Path, mtime: u32) -> Result<(), AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // Windows 的 SetFileTime 要求句柄具备 FILE_WRITE_ATTRIBUTES；只读句柄
        // 会返回权限错误，导致修改时间静默保留失败。
        let file = OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("打开本地文件以设置修改时间失败 {}: {error}", path.display()),
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
