//! 本地下载目标与修改时间管理。
//!
//! 这些函数只处理宿主机文件句柄，不感知 SFTP 协议细节；上传/下载主流程
//! 与 FTP 传输共用同一套断点合法性与收尾语义，集中在这里可避免协议实现
//! 各自复制文件打开与时间保留逻辑。

use std::fs::OpenOptions;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::AsyncSeekExt;
use transom_core::{AppError, ErrorCode, TransferStart};

use crate::sftp::dir::local_io_error;
use crate::sftp::sftp_io_error;

/// 下载收尾：按实际写入字节数截断本地残留尾部，并尽力保留远端修改时间。
///
/// 覆盖模式打开时已经截断，这里的截断等价于取当前长度；续传模式下本地可能
/// 比源更长，多出的旧尾部必须显式裁掉，否则落盘结果比源文件更长。
pub(crate) async fn finish_download_target(
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
pub(crate) async fn open_local_download_target(
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

pub(super) fn unix_seconds(time: Option<SystemTime>) -> Option<u32> {
    let seconds = time?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.min(u64::from(u32::MAX)) as u32)
}
