use crate::client::SshRemote;
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, PreviewMode, RemoteFile, RemoteMetadata};

const TRANSFER_CHUNK: usize = 64 * 1024;

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
