//! SFTP 单文件上传、下载、断点续传、跨远端流转与轻量预览。
//!
//! 传输起点（覆盖或续传）一律由调用方显式声明，本层不再根据目标端已有尺寸
//! 反推断点：目标端“恰好存在同名文件”与“上次中断写了一半”是两件完全不同
//! 的事，靠尺寸猜测会把旧文件当成本次残留，同尺寸直接跳过、更短则把源文件
//! 尾部拼到旧内容之后，最终只能由传输后校验发现脏数据。

mod local;

use self::local::unix_seconds;
pub(crate) use self::local::{finish_download_target, open_local_download_target};
use super::client::{SftpClient, TRANSFER_CHUNK, sftp_error, sftp_io_error};
use super::hash::hex_encode;
use russh_sftp::client::fs::File as RemoteFileHandle;
use russh_sftp::protocol::FileAttributes;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
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
            staging_path,
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

        let write_result: Result<(), AppError> = async {
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
            let _ = remote_file.close().await;
            Ok(())
        }
        .await;

        if let Err(error) = write_result {
            // 写入失败或中途取消：若存在暂存临时文件则尽力清理，原远端目标文件完好无损
            if let Some(staging) = staging_path.as_deref() {
                let _ = self.session.remove_file(staging).await;
            }
            return Err(error);
        }

        self.finish_upload_target(
            remote_path,
            staging_path.as_deref(),
            transferred,
            existing_len,
            local_mtime,
        )
        .await?;
        let local_sha256 = local_hasher.map(|h| hex_encode(&h.finalize()));
        Ok(TransferProgressResult {
            transferred,
            local_sha256,
        })
    }

    /// 上传收尾：将暂存文件原子替换到目标路径，截断续传残留的旧尾部，并尽力保留源文件修改时间。
    ///
    /// 设计原理（Why）：
    /// - 从头覆盖模式下写入的是同目录临时暂存文件（见 `make_staging_path`），收尾时
    ///   通过 `atomic_rename`（优先依赖 russh-sftp 4.0 的 `posix_rename` 扩展）原子替换到
    ///   目标路径；即使覆盖现有文件，也是由操作系统内核单步替换 inode，不仅杜绝了传输
    ///   中断破坏已有文件的隐患，且在远端呈现零中间状态的原子语义；
    /// - 续传模式直接在原文件续写，若远端原有长度超过本次写入长度，显式按实际长度截断旧尾部。
    ///
    /// # Errors
    /// 远端原子重命名或截断失败时返回 `ErrorCode::SftpFailed`；修改时间失败只记日志，
    /// 不影响传输结果的正确性。
    async fn finish_upload_target(
        &self,
        remote_path: &str,
        staging_path: Option<&str>,
        transferred: u64,
        previous_len: u64,
        mtime: Option<u32>,
    ) -> Result<(), AppError> {
        if let Some(staging) = staging_path {
            // 从头覆盖：使用原子重命名把暂存临时文件原子替换为最终目标文件
            if let Err(error) = self.atomic_rename(staging, remote_path).await {
                let _ = self.session.remove_file(staging).await;
                return Err(error);
            }
        } else if previous_len > transferred {
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
    /// 设计原理（Why）：
    /// - 从头覆盖（`requested == 0`）：写入同目录下的 `.transom-upload-*.part` 临时文件，
    ///   绝不直接在目标路径上 `create` 截断，彻底杜绝传输中途取消导致远端正常文件被毁的问题；
    /// - 显式续传（`requested > 0`）：续传的合法性只能由调用方给出的偏移与被覆盖文件的
    ///   当前长度共同保证——远端若已被删除或截短到偏移以下，前缀不再完整，继续追加会写出错位内容，
    ///   此时必须退回从头覆盖（同样采用暂存路径保证安全），而不是报错或照样追加。
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
            let staging = make_staging_path(remote_path);
            let file = self.session.create(&staging).await.map_err(sftp_error)?;
            return Ok(UploadTarget {
                file,
                offset: 0,
                existing_len: 0,
                staging_path: Some(staging),
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
            let staging = make_staging_path(remote_path);
            let file = self.session.create(&staging).await.map_err(sftp_error)?;
            return Ok(UploadTarget {
                file,
                offset: 0,
                existing_len: 0,
                staging_path: Some(staging),
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
            staging_path: None,
        })
    }
}

static UPLOAD_TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

/// 为从头上传生成同目录下的暂存临时文件路径。
///
/// 格式为：`<parent>/.<filename>.transom-upload-<nanos>-<seq>.part`。
fn make_staging_path(remote_path: &str) -> String {
    let (parent, file_name) = match remote_path.rfind('/') {
        Some(idx) => (&remote_path[..idx], &remote_path[idx + 1..]),
        None => ("", remote_path),
    };
    let seq = UPLOAD_TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let token = format!("{now:x}-{seq:x}");
    if parent.is_empty() {
        format!(".{file_name}.transom-upload-{token}.part")
    } else if parent == "/" {
        format!("/.{file_name}.transom-upload-{token}.part")
    } else {
        format!("{parent}/.{file_name}.transom-upload-{token}.part")
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
    /// 从头覆盖时的临时暂存路径；续传模式直接写入目标因此为 None。
    staging_path: Option<String>,
}
