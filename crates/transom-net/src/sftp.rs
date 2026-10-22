use crate::client::SshRemote;
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use transom_core::{AppError, ErrorCode, RemoteFile};

const TRANSFER_CHUNK: usize = 64 * 1024;

#[derive(Clone)]
pub struct SftpClient {
    session: Arc<SftpSession>,
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
        let total = remote
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.size);

        let mut offset = 0u64;
        let mut open_options = tokio::fs::OpenOptions::new();
        open_options.write(true).create(true);

        if resume && local_path.exists() {
            if let Ok(meta) = tokio::fs::metadata(local_path).await {
                let local_size = meta.len();
                if let Some(total_size) = total
                    && local_size >= total_size
                    && total_size > 0
                {
                    progress(total_size, Some(total_size));
                    return Ok(total_size);
                }
                offset = local_size;
                open_options.append(true);
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
        let total = local.metadata().await.ok().map(|metadata| metadata.len());

        let mut offset = 0u64;
        let mut remote_file = if resume {
            if let Ok(meta) = self.session.metadata(remote_path).await {
                let remote_size = meta.size.unwrap_or(0);
                if let Some(total_size) = total
                    && remote_size >= total_size
                    && total_size > 0
                {
                    progress(total_size, Some(total_size));
                    return Ok(total_size);
                }
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
        Ok(transferred)
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        self.session.create_dir(path).await.map_err(sftp_error)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.session.rename(from, to).await.map_err(sftp_error)
    }

    pub async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        if is_dir {
            self.session.remove_dir(path).await.map_err(sftp_error)
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

    pub async fn close(&self) {
        let _ = self.session.close().await;
    }
}

fn sftp_error(error: russh_sftp::client::error::Error) -> AppError {
    AppError::with_detail(ErrorCode::SftpFailed, error.to_string())
}

fn sftp_io_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::SftpFailed, error.to_string())
}
