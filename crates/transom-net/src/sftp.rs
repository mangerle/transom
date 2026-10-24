use crate::client::SshRemote;
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, PreviewMode, RemoteFile, RemoteMetadata, RemoteTextMeta};

const TRANSFER_CHUNK: usize = 64 * 1024;

/// 触发分块并发的最小文件大小：小于它的文件并发收益抵不过多路
/// open/seek 的往返开销，顺序传输即可。
const PARALLEL_THRESHOLD: u64 = 8 * 1024 * 1024;

/// 大文件分块并发的最大块数。
///
/// 单路顺序传输的吞吐被 RTT 锁死（每 64KB 一个同步往返）；并发 4 路
/// 各自独立 handle 与区间，把 RTT 摊平。不再调高：russh 单连接内所有
/// 请求仍共用同一条 TCP 流与加密管线，块数过多只会放大服务器端
/// 并发压力而不继续提升吞吐。
const MAX_PARALLEL_CHUNKS: u64 = 4;

/// 把总字节数均分为不超过 [`MAX_PARALLEL_CHUNKS`] 块，返回 (每块大小, 块数)。
fn chunk_plan(total: u64) -> (u64, usize) {
    let chunk = total.div_ceil(MAX_PARALLEL_CHUNKS).max(1);
    let count = total.div_ceil(chunk).max(1) as usize;
    (chunk, count)
}

/// 单个并发分块的上下文：区间、总量与跨块共享的进度状态。
struct ChunkSpec<'a, F>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    start: u64,
    len: u64,
    total: u64,
    transferred: Arc<AtomicU64>,
    progress: &'a Arc<std::sync::Mutex<F>>,
}

/// 跨并发块共享的进度通知：任一块推进都汇报当前全局累计字节。
fn notify_shared_progress<F>(
    progress: &Arc<std::sync::Mutex<F>>,
    transferred: u64,
    total: Option<u64>,
) where
    F: FnMut(u64, Option<u64>) + Send,
{
    // 回调内部只做同步 try_send，持锁不跨 await；锁中毒（回调 panic）
    // 时放弃该次通知即可，传输本身不受影响。
    if let Ok(mut callback) = progress.lock() {
        callback(transferred, total);
    }
}

#[derive(Clone)]
pub struct SftpClient {
    pub(crate) session: Arc<SftpSession>,
    home: String,
}

impl SftpClient {
    pub async fn open(remote: &SshRemote) -> Result<Self, AppError> {
        let channel = remote.open_session().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;
        let session = SftpSession::new(channel.into_stream())
            .await
            .map_err(sftp_error)?;
        let home = session
            .canonicalize(".")
            .await
            .unwrap_or_else(|_| "/".to_string());
        Ok(Self {
            session: Arc::new(session),
            home,
        })
    }

    pub fn home(&self) -> &str {
        &self.home
    }

    pub async fn list(&self, path: &str) -> Result<(String, Vec<RemoteFile>), AppError> {
        let canonical = self.session.canonicalize(path).await.map_err(sftp_error)?;
        let mut entries = Vec::new();
        for entry in self
            .session
            .read_dir(&canonical)
            .await
            .map_err(sftp_error)?
        {
            let metadata = entry.metadata();
            entries.push(RemoteFile {
                name: entry.file_name(),
                path: entry.path(),
                is_dir: metadata.is_dir(),
                is_symlink: metadata.is_symlink(),
                size: metadata.size.unwrap_or(0),
                modified: metadata.mtime,
                permissions: metadata.permissions,
            });
        }
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
        });
        Ok((canonical, entries))
    }

    /// 读取远端路径的详细属性（lstat 语义，不跟随符号链接）。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：属性查看弹窗需要 uid/gid、访问时间与符号链接目标等完整元数据，
    ///   目录列表条目仅为展示优化的最小字段，故通过独立请求即时获取最新属性；
    /// - 核心优势：采用 lstat 语义可准确识别符号链接本体；链接目标读取失败时
    ///   降级为 `None` 而不中断整体属性展示（错误局部消化）；
    /// - 代价与局限：每次查看属性都会产生额外网络往返，适用于低频交互场景。
    ///
    /// # Errors
    /// 远端路径不存在、权限不足或会话中断时返回 `ErrorCode::SftpFailed`。
    pub async fn stat(&self, path: &str) -> Result<RemoteMetadata, AppError> {
        let metadata = self
            .session
            .symlink_metadata(path)
            .await
            .map_err(sftp_error)?;
        let is_symlink = metadata.is_symlink();
        let link_target = if is_symlink {
            self.session.read_link(path).await.ok()
        } else {
            None
        };
        Ok(RemoteMetadata {
            path: path.to_string(),
            is_dir: metadata.is_dir(),
            is_symlink,
            size: metadata.size.unwrap_or(0),
            uid: metadata.uid,
            user: metadata.user,
            gid: metadata.gid,
            group: metadata.group,
            permissions: metadata.permissions,
            atime: metadata.atime,
            mtime: metadata.mtime,
            link_target,
        })
    }

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

    /// 读取远端文本文件全文，供内置编辑器打开。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：编辑器要拿到整份内容才能就地修改并整体写回。走传输队列会让一次
    ///   应用内部的取文件动作占掉用户的并发传输槽、暴露给队列的取消与清空，因此与
    ///   `read_preview` 同形，做成独立只读通道。
    /// - 核心优势：`max_bytes` 在读取循环内二次设限。进入前虽然已按目录列表的尺寸
    ///   拦过一次，但列表可能过期（文件在列出后变大），循环内设限才真正保证超限内容
    ///   不会被整份吸进内存；UTF-8 校验在边界完成，返回给 UI 的已是合法 `String`，
    ///   上层不必再反复判编码。
    /// - 代价与局限：无进度回调、无校验和，适合上限内的配置与日志片段；更大的文件
    ///   应当走传输队列或在远端编辑。
    ///
    /// # Errors
    /// 路径不存在、权限不足、会话中断、超过 `max_bytes`，或内容不是 UTF-8 时
    /// 返回 `ErrorCode::SftpFailed`。
    ///
    /// 返回值附带读取时刻的版本指纹（实际读到的字节数与读取前 stat 的 mtime），
    /// 供编辑面板保存前做并发冲突比对。
    pub async fn read_text(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> Result<(String, RemoteTextMeta), AppError> {
        let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
        let mut remote = self.session.open(path).await.map_err(sftp_error)?;
        let opened = remote.metadata().await.ok();
        let total = opened.as_ref().and_then(|metadata| metadata.size);
        // mtime 取读取前的 stat：若文件在读取过程中被改写，随后保存前的比对
        // 会因 mtime 不一致命中冲突，恰好覆盖这个竞态窗口。
        let mtime = opened.as_ref().and_then(|metadata| metadata.mtime);
        if let Some(total) = total
            && total > max_bytes
        {
            let _ = remote.close().await;
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("文件 {path} 大小 {total} 字节，超过编辑器上限 {max_bytes} 字节"),
            ));
        }

        let capacity = total
            .map(|size| size.min(max_bytes))
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or(0);
        let mut buffer = Vec::with_capacity(capacity);
        let mut chunk = vec![0u8; TRANSFER_CHUNK];
        loop {
            let count = remote.read(&mut chunk).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            if buffer.len().saturating_add(count) > limit {
                let _ = remote.close().await;
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("文件 {path} 实际大小超过编辑器上限 {max_bytes} 字节，已放弃读取"),
                ));
            }
            buffer.extend_from_slice(&chunk[..count]);
        }
        let _ = remote.close().await;
        // 指纹的字节数取实际读到的长度而非 stat 值：面板持有的是这份内容，
        // 后续比对必须以它为基准才与回传内容一致。
        let meta = RemoteTextMeta {
            size: buffer.len() as u64,
            mtime,
        };

        let content = String::from_utf8(buffer).map_err(|_| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("文件 {path} 不是 UTF-8 文本（可能是二进制或 GBK 等编码），内置编辑器无法安全回写"),
            )
        })?;
        Ok((content, meta))
    }

    /// 将整份文本内容写回远端覆盖，供内置编辑器保存使用；返回远端确认后的
    /// 版本指纹（字节数与 mtime），作为面板下一次比对的基准。
    ///
    /// 设计原理（Why）：
    /// - 必须带 TRUNCATE 打开：SFTP 的写入只覆盖前 N 字节，若编辑后内容比原文件短，
    ///   不截断就会在远端留下旧文件的尾巴，得到一个"新内容 + 残留旧尾巴"的脏文件，
    ///   而这恰好是改配置最常见的操作（删掉几行再保存）。
    /// - `expected` 携带打开时的版本指纹做乐观锁：写前先 stat 比对，远端已被
    ///   他人修改则拒绝覆盖并返回 `ErrorCode::SftpTextConflict`，由面板弹出
    ///   冲突横幅交给用户决策，杜绝静默丢失他人改动。`None` 表示用户已明确
    ///   选择强制覆盖，跳过比对。stat-then-write 非原子，比对与写入之间的窗口
    ///   是该方案的固有限制。
    /// - 写完后重新 stat 比对字节数：截断与部分写是唯一能被廉价发现的失败形态；
    ///   校验和要把整份内容重读一遍，对几百 KB 的配置编辑不划算。
    /// - 代价与局限：非原子操作，写入过程中断会在远端留下半份文件。要做原子替换需
    ///   先写临时名再 rename，而 rename 会改变 inode 并可能破坏正在被进程读取的配置
    ///   文件，属于需要单独权衡的后续改动。
    ///
    /// # Errors
    /// 版本指纹不一致返回 `ErrorCode::SftpTextConflict`；打开、写入、刷新失败，
    /// 或写回后远端大小与提交字节数不一致时返回 `ErrorCode::SftpFailed`。
    pub async fn write_text(
        &self,
        path: &str,
        content: &[u8],
        expected: Option<RemoteTextMeta>,
    ) -> Result<RemoteTextMeta, AppError> {
        if let Some(expected) = expected {
            let metadata = self.session.metadata(path).await.map_err(sftp_error)?;
            if let Some(current) = remote_text_meta_of(metadata.size, metadata.mtime)
                && expected.conflicts_with(&current)
            {
                return Err(AppError::with_detail(
                    ErrorCode::SftpTextConflict,
                    format!(
                        "文件 {path} 自打开后已被修改：当前 {} 字节（mtime {:?}），打开时 {} 字节（mtime {:?}）",
                        current.size, current.mtime, expected.size, expected.mtime
                    ),
                ));
            }
        }
        let mut remote = self
            .session
            .open_with_flags(
                path,
                russh_sftp::protocol::OpenFlags::WRITE
                    | russh_sftp::protocol::OpenFlags::CREATE
                    | russh_sftp::protocol::OpenFlags::TRUNCATE,
            )
            .await
            .map_err(sftp_error)?;
        remote.write_all(content).await.map_err(sftp_io_error)?;
        remote.flush().await.map_err(sftp_io_error)?;
        let expected = content.len() as u64;
        let _ = remote.close().await;

        let metadata = self.session.metadata(path).await.map_err(sftp_error)?;
        let written = metadata.size.ok_or_else(|| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("写回后无法读取远端文件 {path} 的大小，无法确认写入结果"),
            )
        })?;
        if written != expected {
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("远端确认 {written} 字节，提交内容 {expected} 字节，写入可能被中断"),
            ));
        }
        Ok(RemoteTextMeta {
            size: written,
            mtime: metadata.mtime,
        })
    }

    pub async fn download<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        progress: F,
    ) -> Result<u64, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        self.download_resumable(remote_path, local_path, false, progress)
            .await
    }

    pub async fn download_resumable<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        resume: bool,
        mut progress: F,
    ) -> Result<u64, AppError>
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
                        return Ok(total_size);
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

        // 全新的大文件下载走分块并发：单路顺序传输的吞吐被 RTT 锁死
        // （50ms 链路约 1.3MB/s），多路独立 handle 各传一段把往返摊平。
        // 续传（offset > 0）与未知大小的文件保持顺序路径。
        if total.is_some_and(|size| size >= PARALLEL_THRESHOLD) && offset == 0 {
            let total = total.unwrap_or_default();
            // 开头为取元数据打开的 handle 在并行路径里不再使用，先归还。
            let _ = remote.close().await;
            let result = self
                .download_parallel(remote_path, local_path, total, progress)
                .await;
            if result.is_ok()
                && let Some(mtime) = mtime
            {
                let _ = set_local_mtime(local_path, mtime).await;
            }
            return result.map(|_| total);
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

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut transferred = offset;
        progress(transferred, total);

        loop {
            let count = remote.read(&mut buffer).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            local
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            transferred += count as u64;
            progress(transferred, total);
        }
        local.flush().await.map_err(sftp_io_error)?;
        let _ = remote.close().await;
        if let Some(mtime) = mtime
            && let Err(error) = set_local_mtime(local_path, mtime).await
        {
            tracing::debug!(
                path = %local_path.display(),
                %error,
                "保留本地文件修改时间失败"
            );
        }
        Ok(transferred)
    }

    pub async fn upload<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress: F,
    ) -> Result<u64, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        self.upload_resumable(local_path, remote_path, false, progress)
            .await
    }

    pub async fn upload_resumable<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        resume: bool,
        mut progress: F,
    ) -> Result<u64, AppError>
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

        // 全新的大文件上传同样走分块并发（与下载同因：RTT 摊平）。
        // 续传路径保持顺序。
        if !resume && total.is_some_and(|size| size >= PARALLEL_THRESHOLD) {
            let total = total.unwrap_or_default();
            let result = self
                .upload_parallel(local_path, remote_path, total, progress)
                .await;
            if result.is_ok()
                && let Some(mtime) = local_mtime
            {
                let _ = self.set_mtime(remote_path, mtime).await;
            }
            return result.map(|_| total);
        }

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
                        return Ok(total_size);
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

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut transferred = offset;
        progress(transferred, total);

        loop {
            let count = local.read(&mut buffer).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            remote_file
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            transferred += count as u64;
            progress(transferred, total);
        }
        remote_file.shutdown().await.map_err(sftp_io_error)?;
        if let Some(mtime) = local_mtime
            && let Err(error) = self.set_mtime(remote_path, mtime).await
        {
            tracing::debug!(path = %remote_path, %error, "保留远端文件修改时间失败");
        }
        Ok(transferred)
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        self.session.create_dir(path).await.map_err(sftp_error)
    }

    /// 递归创建远端目录，已存在的目录视为成功。
    ///
    /// 设计原理（Why）：SFTP 只有单层 MKDIR，没有 `mkdir -p`；目录传输需要
    /// 逐级创建。逐段调用并校验失败后目录是否真实存在，可避免把“已存在”
    /// 与权限不足、磁盘故障等真实错误混为一谈。
    pub(crate) async fn create_dir_all(&self, path: &str) -> Result<(), AppError> {
        if path.is_empty() || path == "/" || path == "." {
            return Ok(());
        }
        let absolute = path.starts_with('/');
        let mut current = String::new();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            if absolute || !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            self.ensure_dir(&current).await?;
        }
        Ok(())
    }

    /// 确保单个远端目录存在；创建失败时通过属性确认是否已存在同名目录。
    pub(crate) async fn ensure_dir(&self, path: &str) -> Result<(), AppError> {
        if let Err(error) = self.session.create_dir(path).await {
            let exists_as_dir = self
                .session
                .metadata(path)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            if !exists_as_dir {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("创建远端目录失败 {path}: {error}"),
                ));
            }
        }
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.session.rename(from, to).await.map_err(sftp_error)
    }

    /// 创建远端软链接：`target` 为被指向的既有路径，`link_path` 为新建链接自身路径。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：`SSH_FXP_SYMLINK` 报文内两个路径参数的先后顺序存在历史分歧——
    ///   OpenSSH 服务端将首个字符串解释为 targetpath（被指向目标）、次个为 linkpath
    ///   （新建链接），与协议草案的字段声明顺序恰好相反；主流服务端均遵循 OpenSSH 行为；
    /// - 核心优势：在客户端封装层统一消化该分歧，参数语义以本方法签名为准，
    ///   上层无需感知报文细节，杜绝链接方向被静默颠倒；
    /// - 代价与局限：面向严格遵循协议草案的少数非 OpenSSH 服务端时链接方向会相反，
    ///   当前以事实标准（OpenSSH）为准；russh-sftp 客户端形参命名沿用草案顺序，
    ///   因此这里对调传入，实际报文顺序为 (target, link_path)。
    ///
    /// # Errors
    /// 链接目标不存在、链接路径已存在或权限不足时返回 `ErrorCode::SftpFailed`。
    pub async fn symlink(&self, target: &str, link_path: &str) -> Result<(), AppError> {
        self.session
            .symlink(target, link_path)
            .await
            .map_err(sftp_error)
    }

    /// 删除远端文件或目录。
    ///
    /// SFTP 的 RMDIR 只能删除空目录，目录分支因此改为先收集并删除全部后代，
    /// 再自底向上删除目录本身，语义对齐本地文件管理器的“删除目录”。
    pub async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        if is_dir {
            self.delete_dir_recursive(path).await
        } else {
            self.session.remove_file(path).await.map_err(sftp_error)
        }
    }

    /// 修改远程文件或目录的 POSIX 权限位（低 9 位生效，保留原有文件类型高位）
    pub async fn chmod(&self, path: &str, permissions: u32) -> Result<(), AppError> {
        let mut meta = self
            .session
            .metadata(path)
            .await
            .unwrap_or_else(|_| russh_sftp::protocol::FileAttributes::empty());
        let high_bits = meta.permissions.unwrap_or(0) & !0o777;
        meta.permissions = Some(high_bits | (permissions & 0o777));
        self.session
            .set_metadata(path, meta)
            .await
            .map_err(sftp_error)
    }

    /// 仅设置远端文件的修改时间，不触碰权限与其他属性。
    ///
    /// 目录同步依赖两端 mtime 判断新旧；若上传后不保留源文件时间，
    /// 下一次比较会把刚上传的文件误判为对端更新，从而反复回传。
    ///
    /// # Errors
    /// 远端路径不存在、权限不足或服务端不支持 SETSTAT 时返回
    /// `ErrorCode::SftpFailed`；调用方通常以降级方式处理该错误。
    pub async fn set_mtime(&self, path: &str, mtime: u32) -> Result<(), AppError> {
        let mut attributes = russh_sftp::protocol::FileAttributes::empty();
        attributes.mtime = Some(mtime);
        self.session
            .set_metadata(path, attributes)
            .await
            .map_err(sftp_error)
    }

    /// 分块并发下载：每块独立远端 handle 与本地写入区间，进度按全局累计汇报。
    ///
    /// 任一块失败即整体失败（`try_join_all` 取消其余块），本地留下半成品，
    /// 与顺序路径的失败语义一致，可由续传机制接续。
    async fn download_parallel<F>(
        &self,
        remote_path: &str,
        local_path: &Path,
        total: u64,
        progress: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let (chunk, count) = chunk_plan(total);
        let transferred = Arc::new(AtomicU64::new(0));
        let progress = Arc::new(std::sync::Mutex::new(progress));
        // 先以截断方式创建一次干净目标，各块随后只写自己的区间。
        let file = tokio::fs::File::create(local_path).await.map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("打开本地文件失败 {}: {error}", local_path.display()),
            )
        })?;
        drop(file);

        let mut chunks = Vec::with_capacity(count);
        for index in 0..count {
            let start = index as u64 * chunk;
            let len = chunk.min(total - start);
            chunks.push(self.download_chunk(
                remote_path,
                local_path,
                ChunkSpec {
                    start,
                    len,
                    total,
                    transferred: transferred.clone(),
                    progress: &progress,
                },
            ));
        }
        futures::future::try_join_all(chunks)
            .await
            .map_err(|error: AppError| error)?;
        Ok(())
    }

    /// 下载单个分块：独立 handle 顺序读取区间并写入本地对应偏移。
    async fn download_chunk(
        &self,
        remote_path: &str,
        local_path: &Path,
        spec: ChunkSpec<'_, impl FnMut(u64, Option<u64>) + Send>,
    ) -> Result<(), AppError> {
        let ChunkSpec {
            start,
            len,
            total,
            transferred,
            progress,
        } = spec;
        let mut remote = self.session.open(remote_path).await.map_err(sftp_error)?;
        let mut local = tokio::fs::OpenOptions::new()
            .write(true)
            .open(local_path)
            .await
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("打开本地文件失败 {}: {error}", local_path.display()),
                )
            })?;
        if start > 0 {
            remote
                .seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(sftp_io_error)?;
            local
                .seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(sftp_io_error)?;
        }

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut done = 0u64;
        while done < len {
            let want = (len - done).min(TRANSFER_CHUNK as u64) as usize;
            let count = remote
                .read(&mut buffer[..want])
                .await
                .map_err(sftp_io_error)?;
            if count == 0 {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("远端在 {start}+{done} 处提前结束，文件 {remote_path} 可能正被改写"),
                ));
            }
            local
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            done += count as u64;
            let current = transferred.fetch_add(count as u64, Ordering::Relaxed) + count as u64;
            notify_shared_progress(progress, current, Some(total));
        }
        let _ = remote.close().await;
        Ok(())
    }

    /// 分块并发上传：先截断清空远端目标，再各块独立 handle 写各自区间。
    async fn upload_parallel<F>(
        &self,
        local_path: &Path,
        remote_path: &str,
        total: u64,
        progress: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let (chunk, count) = chunk_plan(total);
        let transferred = Arc::new(AtomicU64::new(0));
        let progress = Arc::new(std::sync::Mutex::new(progress));
        // 并发块不能各自带 TRUNCATE（会互相清掉已写数据）：
        // 先 create 一次完成截断，各块以 WRITE|CREATE 追加语义写区间。
        let truncate = self.session.create(remote_path).await.map_err(sftp_error)?;
        let _ = truncate.close().await;

        let mut chunks = Vec::with_capacity(count);
        for index in 0..count {
            let start = index as u64 * chunk;
            let len = chunk.min(total - start);
            chunks.push(self.upload_chunk(
                local_path,
                remote_path,
                ChunkSpec {
                    start,
                    len,
                    total,
                    transferred: transferred.clone(),
                    progress: &progress,
                },
            ));
        }
        futures::future::try_join_all(chunks)
            .await
            .map_err(|error: AppError| error)?;
        Ok(())
    }

    /// 上传单个分块：读本地区间偏移写入远端对应区间。
    async fn upload_chunk(
        &self,
        local_path: &Path,
        remote_path: &str,
        spec: ChunkSpec<'_, impl FnMut(u64, Option<u64>) + Send>,
    ) -> Result<(), AppError> {
        let ChunkSpec {
            start,
            len,
            total,
            transferred,
            progress,
        } = spec;
        let mut local = tokio::fs::File::open(local_path).await.map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("打开本地文件失败 {}: {error}", local_path.display()),
            )
        })?;
        let mut remote = self
            .session
            .open_with_flags(
                remote_path,
                russh_sftp::protocol::OpenFlags::WRITE | russh_sftp::protocol::OpenFlags::CREATE,
            )
            .await
            .map_err(sftp_error)?;
        if start > 0 {
            local
                .seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(sftp_io_error)?;
            remote
                .seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(sftp_io_error)?;
        }

        let mut buffer = vec![0u8; TRANSFER_CHUNK];
        let mut done = 0u64;
        while done < len {
            let want = (len - done).min(TRANSFER_CHUNK as u64) as usize;
            let count = local
                .read(&mut buffer[..want])
                .await
                .map_err(sftp_io_error)?;
            if count == 0 {
                return Err(AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("本地文件 {remote_path} 在读取途中变短，无法继续上传"),
                ));
            }
            remote
                .write_all(&buffer[..count])
                .await
                .map_err(sftp_io_error)?;
            done += count as u64;
            let current = transferred.fetch_add(count as u64, Ordering::Relaxed) + count as u64;
            notify_shared_progress(progress, current, Some(total));
        }
        remote.shutdown().await.map_err(sftp_io_error)?;
        let _ = remote.close().await;
        Ok(())
    }

    pub async fn close(&self) {
        let _ = self.session.close().await;
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

pub(crate) fn sftp_error(error: russh_sftp::client::error::Error) -> AppError {
    AppError::with_detail(ErrorCode::SftpFailed, error.to_string())
}

pub(crate) fn sftp_io_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::SftpFailed, error.to_string())
}

/// 把远端 stat 结果折算成版本指纹；缺少字节数时返回 `None`，调用方据此跳过
/// 比对而不是误判冲突（无法验证不等于发生了冲突）。只取两个所需字段而非
/// 整个 stat 对象，避免为一次折算引入协议层类型依赖。
fn remote_text_meta_of(size: Option<u64>, mtime: Option<u32>) -> Option<RemoteTextMeta> {
    Some(RemoteTextMeta { size: size?, mtime })
}

#[cfg(test)]
mod tests {
    use super::{MAX_PARALLEL_CHUNKS, chunk_plan};

    /// 分块必须完整覆盖文件且块数受上限约束：漏区间会静默丢数据，
    /// 块数越界则放大服务器并发压力。
    #[test]
    fn chunk_plan_covers_exactly_within_bounds() {
        for total in [
            1u64,
            1024,
            8 * 1024 * 1024,
            8 * 1024 * 1024 + 1,
            100 * 1024 * 1024,
            u32::MAX as u64,
        ] {
            let (chunk, count) = chunk_plan(total);
            assert!(count >= 1, "至少一块: total={total}");
            assert!(
                count as u64 <= MAX_PARALLEL_CHUNKS,
                "块数受上限: total={total}"
            );
            let covered = chunk * count as u64;
            assert!(
                covered >= total,
                "完整覆盖: total={total}, covered={covered}"
            );
            // 每块都不空（最后一块可能较短）
            assert!(
                chunk >= total.div_ceil(MAX_PARALLEL_CHUNKS),
                "块大小不小于均分值"
            );
        }
    }
}
