//! FTP 单文件与目录传输：REST 断点续传、流式落盘与递归目录同步。
//!
//! 设计原理（Why）：
//! - 传输起点（覆盖或续传）由调用方显式声明，与 SFTP 侧共用同一套起点语义
//!   与本地目标打开/收尾逻辑（`sftp::transfer` 的本地助手），两侧断点行为
//!   完全一致，面板不需要为协议差异做任何适配；
//! - 下载用 `REST`+`RETR` 从偏移续读，上传用 `REST`+`STOR` 从偏移覆写；
//!   FTP 没有"按长度截断远端文件"的协议能力（SFTP 用 set_metadata 收尾
//!   截断），续传后若远端旧尾部比本次写入更长，残留无法在协议层消除，
//!   这是 FTP 续传的固有限制，与其他主流 FTP 工具行为一致；
//! - 传输体直接持有独立连接的会话锁顺序执行（不走交互路径的闭包封装），
//!   哈希器、本地句柄与进度回调都以普通所有权参与，取消任务（tokio abort）
//!   时锁与连接整体释放，交互连接不受任何影响。

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{
    AppError, ErrorCode, TransferFilter, TransferMatcher, TransferStart, TransferSummary,
};

use super::FtpClient;
use crate::sftp::hash::hex_encode;
use crate::sftp::transfer::{
    TransferProgressResult, finish_download_target, open_local_download_target,
};
use crate::sftp::{TRANSFER_CHUNK, dir::local_io_error};

impl FtpClient {
    /// 顺序下载远端文件，按调用方声明的起点续传。
    ///
    /// 断点语义与 SFTP 侧一致：`Overwrite` 截断重写并同步计算本地 SHA-256；
    /// `Continue{offset}` 要求偏移落在远端文件长度之内且本地长度覆盖偏移，
    /// 任一条件不满足退回从头覆盖。传输在独立连接上进行。
    ///
    /// # Errors
    /// 打开远端或本地文件、读取、写入失败时返回 `ErrorCode::FtpFailed`；
    /// 保留本地修改时间失败只记录调试日志，不影响传输结果。
    pub async fn download_resumable<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        start: TransferStart,
        progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let transfer = self.open_transfer_client().await?;
        Self::download_resumable_on(&transfer, remote_path, local_path, start, progress).await
    }

    /// 在指定传输连接上执行断点续传下载；目录传输复用同一连接逐文件调用。
    pub(crate) async fn download_resumable_on<F>(
        transfer: &FtpClient,
        remote_path: &str,
        local_path: &Path,
        start: TransferStart,
        mut progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let total = transfer.size_optional(remote_path).await?;
        let (mut local, offset) = open_local_download_target(local_path, start, total).await?;
        let mtime = transfer.mtime_optional(remote_path).await?;

        let mut session = transfer.lock_session().await;
        transfer.ensure_connected(&mut session).await?;
        let stream = session
            .stream
            .as_mut()
            .ok_or_else(transfer_stream_missing)?;

        let mut hasher = if offset == 0 {
            Some(Sha256::new())
        } else {
            None
        };
        let mut transferred = offset;
        progress(transferred, total);

        let mut data = stream.download_stream(remote_path, offset).await?;
        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        loop {
            let count = data.read(&mut buffer).await.map_err(transfer_io_error)?;
            if count == 0 {
                break;
            }
            if let Some(hasher) = hasher.as_mut() {
                hasher.update(&buffer[..count]);
            }
            local
                .write_all(&buffer[..count])
                .await
                .map_err(|error| local_io_error("写入本地文件失败", local_path, error))?;
            transferred = transferred.saturating_add(count as u64);
            progress(transferred, total);
        }
        data.finish().await?;
        drop(session);

        finish_download_target(local_path, &mut local, transferred, mtime).await?;
        let local_sha256 = hasher.map(|digest| hex_encode(&digest.finalize()));
        Ok(TransferProgressResult {
            transferred,
            local_sha256,
        })
    }

    /// 顺序上传本地文件，按调用方声明的起点续传。
    ///
    /// `Continue{offset}` 要求偏移落在源文件长度之内且远端已有长度覆盖偏移，
    /// 否则退回从头覆盖；`REST`+`STOR` 从偏移处覆写。FTP 无远端截断能力，
    /// 续传后远端残留旧尾部无法在协议层消除（见模块文档）。
    ///
    /// # Errors
    /// 本地读取或远端写入失败时返回 `ErrorCode::FtpFailed`。
    pub async fn upload_resumable<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        start: TransferStart,
        progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let transfer = self.open_transfer_client().await?;
        Self::upload_resumable_on(&transfer, local_path, remote_path, start, progress).await
    }

    /// 在指定传输连接上执行断点续传上传；目录传输复用同一连接逐文件调用。
    pub(crate) async fn upload_resumable_on<F>(
        transfer: &FtpClient,
        local_path: &Path,
        remote_path: &str,
        start: TransferStart,
        mut progress: F,
    ) -> Result<TransferProgressResult, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let source = tokio::fs::metadata(local_path)
            .await
            .map_err(|error| local_io_error("读取本地文件信息失败", local_path, error))?;
        let source_len = source.len();
        let mut offset = match start {
            TransferStart::Continue { offset } if offset <= source_len && offset > 0 => offset,
            _ => 0,
        };
        if offset > 0 {
            let existing = transfer.size_optional(remote_path).await?.unwrap_or(0);
            if existing < offset {
                log::debug!("远端文件不足以支撑续传偏移，退回从头覆盖，路径: {remote_path}");
                offset = 0;
            }
        }

        let mut local = tokio::fs::File::open(local_path)
            .await
            .map_err(|error| local_io_error("打开本地文件失败", local_path, error))?;
        local
            .seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|error| local_io_error("定位本地文件偏移失败", local_path, error))?;

        let mut session = transfer.lock_session().await;
        transfer.ensure_connected(&mut session).await?;
        let stream = session
            .stream
            .as_mut()
            .ok_or_else(transfer_stream_missing)?;

        let mut hasher = if offset == 0 {
            Some(Sha256::new())
        } else {
            None
        };
        let mut transferred = offset;
        progress(transferred, Some(source_len));

        let mut data = stream.upload_stream(remote_path, offset).await?;
        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        loop {
            let count = local
                .read(&mut buffer)
                .await
                .map_err(|error| local_io_error("读取本地文件失败", local_path, error))?;
            if count == 0 {
                break;
            }
            if let Some(hasher) = hasher.as_mut() {
                hasher.update(&buffer[..count]);
            }
            data.write_all(&buffer[..count])
                .await
                .map_err(transfer_io_error)?;
            transferred = transferred.saturating_add(count as u64);
            progress(transferred, Some(source_len));
        }
        data.finish().await?;
        drop(session);

        let local_sha256 = hasher.map(|digest| hex_encode(&digest.finalize()));
        Ok(TransferProgressResult {
            transferred,
            local_sha256,
        })
    }

    /// 递归上传本地目录（顺序逐文件），返回整体字节数与跳过条目数。
    ///
    /// 排除规则在扫描阶段对目录直接剪枝；文件以覆盖语义逐个写入（目录任务
    /// 没有"整棵树已传字节数"，目标端同名文件属于本次要覆盖的旧版本）。
    /// 整个目录复用一条传输连接，海量小文件不产生逐文件握手的开销。
    ///
    /// # Errors
    /// 排除规则非法、本地目录不可读、远端建目录或文件上传失败时返回
    /// `ErrorCode::Configuration` 或 `ErrorCode::FtpFailed`。符号链接会被跳过。
    pub async fn upload_dir<F>(
        &self,
        local_root: &Path,
        remote_root: &str,
        filter: &TransferFilter,
        progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let matcher = filter.matcher()?;
        let entries = scan_local_dir(local_root, &matcher).await?;
        let total = entries
            .iter()
            .fold(0u64, |total, entry| total.saturating_add(entry.size));

        // 先按相对路径建齐目录，保证空目录也能同步。目录按路径深度排序，
        // 父段必然先于子段创建；远端根目录本身可能尚不存在，同样纳入确保。
        self.ensure_remote_dir(remote_root).await?;
        let mut directories: Vec<&ScanEntry> =
            entries.iter().filter(|entry| entry.is_dir).collect();
        directories.sort_by_key(|entry| entry.relative_path.matches('/').count());
        for entry in directories {
            let target = crate::ftp::listing::join_remote_path(remote_root, &entry.relative_path);
            self.ensure_remote_dir(&target).await?;
        }
        let transfer = self.open_transfer_client().await?;
        let progress = Arc::new(Mutex::new(progress));
        let mut transferred = 0u64;
        for entry in entries.iter().filter(|entry| !entry.is_dir) {
            let source = local_root.join(&entry.relative_path);
            let target = crate::ftp::listing::join_remote_path(remote_root, &entry.relative_path);
            if let Some(parent) = Path::new(&target).parent() {
                self.ensure_remote_dir(&parent.to_string_lossy()).await?;
            }
            let shared = Arc::clone(&progress);
            transferred = transferred.saturating_add(
                Self::upload_resumable_on(
                    &transfer,
                    &source,
                    &target,
                    TransferStart::Overwrite,
                    move |done, done_total| {
                        shared.lock()(done, done_total);
                    },
                )
                .await?
                .transferred,
            );
            progress.lock()(transferred, Some(total));
        }
        Ok(TransferSummary {
            transferred,
            skipped: 0,
            verified: false,
        })
    }

    /// 递归下载远端目录（顺序逐文件），返回整体字节数与跳过条目数。
    ///
    /// 远端扫描用显式栈逐层展开（老服务器退化 LIST）；本地目录按相对路径
    /// 逐级创建，整个目录复用一条传输连接。
    ///
    /// # Errors
    /// 排除规则非法、远端读取失败或本地建目录/写入失败时返回
    /// `ErrorCode::Configuration` 或 `ErrorCode::FtpFailed`。
    pub async fn download_dir<F>(
        &self,
        remote_root: &str,
        local_root: &Path,
        filter: &TransferFilter,
        progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let matcher = filter.matcher()?;
        let entries = self.scan_remote_dir(remote_root).await?;
        // 过滤在扫描后统一应用：`matches` 返回 true 表示该条目被排除。
        let included = |entry: &&ScanEntry| !matcher.matches(&entry.relative_path, entry.is_dir);
        let total = entries
            .iter()
            .filter(included)
            .fold(0u64, |total, entry| total.saturating_add(entry.size));

        tokio::fs::create_dir_all(local_root)
            .await
            .map_err(|error| local_io_error("创建本地根目录失败", local_root, error))?;
        for entry in entries
            .iter()
            .filter(|entry| entry.is_dir && included(entry))
        {
            let target = local_root.join(&entry.relative_path);
            tokio::fs::create_dir_all(&target)
                .await
                .map_err(|error| local_io_error("创建本地目录失败", &target, error))?;
        }

        let transfer = self.open_transfer_client().await?;
        let progress = Arc::new(Mutex::new(progress));
        let mut transferred = 0u64;
        for entry in entries
            .iter()
            .filter(|entry| !entry.is_dir && included(entry))
        {
            let source = crate::ftp::listing::join_remote_path(remote_root, &entry.relative_path);
            let target = local_root.join(&entry.relative_path);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| local_io_error("创建本地目录失败", parent, error))?;
            }
            let shared = Arc::clone(&progress);
            transferred = transferred.saturating_add(
                Self::download_resumable_on(
                    &transfer,
                    &source,
                    &target,
                    TransferStart::Overwrite,
                    move |done, done_total| {
                        shared.lock()(done, done_total);
                    },
                )
                .await?
                .transferred,
            );
            progress.lock()(transferred, Some(total));
        }
        Ok(TransferSummary {
            transferred,
            skipped: 0,
            verified: false,
        })
    }

    /// 确保远端目录存在：先尝试 MKD，失败后核实是否为"已存在"误报。
    ///
    /// FTP 的 MKD 对已存在目录返回 5xx，但没有标准错误码区分"已存在"与
    /// 权限不足，只能用属性查询复核；复核也失败才把原始错误抛出。
    async fn ensure_remote_dir(&self, path: &str) -> Result<(), AppError> {
        match self.create_dir(path).await {
            Ok(()) => Ok(()),
            Err(error) => match self.stat(path).await {
                Ok(metadata) if metadata.is_dir => Ok(()),
                _ => Err(error),
            },
        }
    }

    /// 递归扫描远端目录树：显式栈逐层展开，输出按相对路径排序的条目。
    ///
    /// 列表条目已在列表层完成安全文件名校验，相对路径拼接不会越界；
    /// 符号链接按文件处理不跟随。
    async fn scan_remote_dir(&self, root: &str) -> Result<Vec<ScanEntry>, AppError> {
        let mut entries = Vec::new();
        let mut pending = vec![(root.to_string(), String::new())];
        while let Some((directory, prefix)) = pending.pop() {
            let (_, children) = self.list(&directory).await?;
            for child in children {
                let relative = if prefix.is_empty() {
                    child.name.clone()
                } else {
                    format!("{prefix}/{}", child.name)
                };
                if child.is_dir {
                    pending.push((child.path.clone(), relative.clone()));
                }
                entries.push(ScanEntry {
                    relative_path: relative,
                    is_dir: child.is_dir,
                    size: child.size,
                });
            }
        }
        // 栈的弹出顺序会打乱层级，按相对路径重排保证输出稳定。
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(entries)
    }
}

/// 目录扫描条目：相对路径与最小元数据。
struct ScanEntry {
    relative_path: String,
    is_dir: bool,
    size: u64,
}

/// 递归扫描本地目录树：显式栈防爆栈，排除规则对目录直接剪枝。
async fn scan_local_dir(
    root: &Path,
    matcher: &TransferMatcher,
) -> Result<Vec<ScanEntry>, AppError> {
    let mut entries = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        let mut children = tokio::fs::read_dir(&directory)
            .await
            .map_err(|error| local_io_error("读取本地目录失败", &directory, error))?;
        while let Some(child) = children
            .next_entry()
            .await
            .map_err(|error| local_io_error("读取本地目录条目失败", &directory, error))?
        {
            let name = child.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let metadata = child
                .metadata()
                .await
                .map_err(|error| local_io_error("读取本地文件信息失败", &directory, error))?;
            if metadata.is_symlink() {
                continue;
            }
            // `matches` 返回 true 表示排除：被排除的目录不递归、文件不入清单。
            if metadata.is_dir() {
                if !matcher.matches(&relative, true) {
                    pending.push((directory.join(&name), relative.clone()));
                }
                entries.push(ScanEntry {
                    relative_path: relative,
                    is_dir: true,
                    size: 0,
                });
            } else if !matcher.matches(&relative, false) {
                entries.push(ScanEntry {
                    relative_path: relative,
                    is_dir: false,
                    size: metadata.len(),
                });
            }
        }
    }
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(entries)
}

/// 传输会话未连接时的兜底错误（正常路径中 ensure_connected 已保证）。
fn transfer_stream_missing() -> AppError {
    AppError::with_detail(ErrorCode::FtpFailed, "FTP 传输连接未能建立")
}

/// 传输数据流的底层 IO 故障统一转为领域错误。
fn transfer_io_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::FtpFailed, format!("FTP 数据传输失败: {error}"))
}
