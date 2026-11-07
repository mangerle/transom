//! 归档 tar 的流式读写、临时文件落盘与进度上报。
use super::TRANSFER_CHUNK;
use crate::bridge::{BlockingReader, output_channel};
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 阻塞地把原始 tar 流写成文件，成功后原子改名。
pub(super) fn write_archive_to_file(
    mut reader: BlockingReader,
    container_path: &str,
    destination: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let temp = temp_path(destination);
    let mut file = std::fs::File::create(&temp).map_err(|error| ContainerError::File {
        path: temp.display().to_string(),
        message: format!("创建临时归档失败: {error}"),
    })?;
    let mut buffer = vec![0u8; TRANSFER_CHUNK];
    let mut written = 0u64;
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(ContainerError::Cancelled);
        }
        let count = match reader.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(count) => count,
            Err(error) => {
                break Err(ContainerError::File {
                    path: container_path.to_string(),
                    message: format!("读取目录归档失败: {error}"),
                });
            }
        };
        if let Err(error) = file.write_all(&buffer[..count]) {
            break Err(ContainerError::File {
                path: temp.display().to_string(),
                message: format!("写入目录归档失败: {error}"),
            });
        }
        written = written.saturating_add(count as u64);
        let _ = progress.try_send((written, None));
    };
    if result.is_ok() {
        let _ = file.sync_all();
    }
    drop(file);
    match result {
        Ok(()) => {
            std::fs::rename(&temp, destination).map_err(|error| ContainerError::File {
                path: destination.display().to_string(),
                message: format!("重命名目录归档失败: {error}"),
            })?;
            Ok(written)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// 容器内已有文件的元数据快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArchiveMetadata {
    pub(super) mode: u32,
    pub(super) uid: u64,
    pub(super) gid: u64,
    pub(super) mtime: u64,
}

/// 从归档流读取首个条目头；不读取正文，读完头即可停止。
pub(super) fn metadata_from_archive<R: Read>(reader: R) -> Option<ArchiveMetadata> {
    let mut archive = tar::Archive::new(reader);
    let mut entries = archive.entries().ok()?;
    let entry = entries.next()?.ok()?;
    let header = entry.header();
    Some(ArchiveMetadata {
        mode: header.mode().ok()?,
        uid: header.uid().ok()?,
        gid: header.gid().ok()?,
        mtime: header.mtime().ok()?,
    })
}

/// 读取已有文件元数据；文件不存在或引擎不支持时返回 `None`，由调用方使用默认值。
pub(super) async fn read_existing_metadata(
    api: &crate::api::DockerApi,
    container: &str,
    path: &str,
) -> Option<ArchiveMetadata> {
    let (reader, task) = match open_archive(api, container, path).await {
        Ok(archive) => archive,
        Err(ContainerError::NotFound { .. }) => return None,
        Err(error) => {
            log::debug!("读取容器文件元数据失败，路径: {path}，原因: {error}");
            return None;
        }
    };
    let metadata = tokio::task::spawn_blocking(move || metadata_from_archive(reader))
        .await
        .ok()
        .flatten();
    task.abort();
    metadata
}

/// 打开归档流：后台任务把 HTTP 响应体泵入有界通道，阻塞侧按需读取。
pub(crate) async fn open_archive(
    api: &crate::api::DockerApi,
    container: &str,
    path: &str,
) -> Result<(BlockingReader, tokio::task::JoinHandle<()>), ContainerError> {
    let suffix = format!(
        "/containers/{}/archive?path={}",
        http::percent_encode(container),
        http::percent_encode(path)
    );
    let response = http::request(
        api.transport_ref(),
        hyper::Method::GET,
        &api.versioned_path(&suffix),
        Bytes::new(),
        None,
    )
    .await?;
    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        if status == hyper::StatusCode::NOT_FOUND {
            return Err(ContainerError::NotFound {
                target: path.to_string(),
            });
        }
        return Err(ContainerError::Api {
            operation: format!("读取容器归档 {path}"),
            status: status.as_u16(),
            body,
        });
    }
    let (sender, reader) = output_channel(8);
    // 整个 response 交给泵任务：只搬 body 会提前中止连接驱动，归档流会被截断。
    let task = http::pump_body(response, sender);
    Ok((reader, task))
}

pub(super) fn download_to_file(
    reader: BlockingReader,
    path: &str,
    destination: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let temp = temp_path(destination);
    let result = (|| -> Result<u64, ContainerError> {
        let mut archive = tar::Archive::new(reader);
        let mut entries = archive.entries().map_err(|error| ContainerError::File {
            path: path.to_string(),
            message: format!("读取 tar 失败: {error}"),
        })?;
        let mut file = std::fs::File::create(&temp).map_err(|error| ContainerError::File {
            path: temp.display().to_string(),
            message: error.to_string(),
        })?;
        let mut written = 0u64;
        let mut found = false;
        for entry in entries.by_ref() {
            let mut entry = entry.map_err(|error| ContainerError::File {
                path: path.to_string(),
                message: format!("解析 tar 条目失败: {error}"),
            })?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            found = true;
            let total = entry.header().size().unwrap_or(0);
            let mut buffer = vec![0u8; TRANSFER_CHUNK];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(ContainerError::Cancelled);
                }
                let count = entry
                    .read(&mut buffer)
                    .map_err(|error| ContainerError::File {
                        path: path.to_string(),
                        message: format!("读取文件内容失败: {error}"),
                    })?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])
                    .map_err(|error| ContainerError::File {
                        path: temp.display().to_string(),
                        message: error.to_string(),
                    })?;
                written = written.saturating_add(count as u64);
                let _ = progress.try_send((written, Some(total)));
            }
            break;
        }
        if !found {
            return Err(ContainerError::NotFound {
                target: path.to_string(),
            });
        }
        file.sync_all().map_err(|error| ContainerError::File {
            path: temp.display().to_string(),
            message: error.to_string(),
        })?;
        drop(file);
        std::fs::rename(&temp, destination).map_err(|error| ContainerError::File {
            path: destination.display().to_string(),
            message: error.to_string(),
        })?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// 临时文件与目标同目录，保证 rename 是同一文件系统内的原子操作。
pub(super) fn temp_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    destination.with_file_name(format!(".{name}.{stamp}.transom-part"))
}

/// 递归构建目录 tar，写入有界通道；发送端阻塞形成反压。
pub(super) fn build_dir_tar_stream(
    local_dir: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
) -> Result<u64, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".to_string());
    }
    let writer = ProgressChannelWriter {
        sender,
        written: 0,
        progress: progress.clone(),
    };
    let mut builder = tar::Builder::new(writer);
    builder.mode(tar::HeaderMode::Deterministic);
    builder.follow_symlinks(false);
    builder
        .append_dir_all(".", local_dir)
        .map_err(|error| error.to_string())?;
    builder.finish().map_err(|error| error.to_string())?;
    let writer = builder.into_inner().map_err(|error| error.to_string())?;
    Ok(writer.written)
}

pub(super) fn build_tar_stream(
    local_path: &Path,
    name: &str,
    total: u64,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
) -> Result<(), String> {
    let file = std::fs::File::open(local_path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    let mut header = tar::Header::new_gnu();
    header.set_size(total);
    header.set_mode(file_mode(&metadata));
    header.set_mtime(
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
    );
    header.set_cksum();
    let reader = ProgressReader {
        inner: file,
        read: 0,
        total,
        cancel: Arc::clone(cancel),
        progress: progress.clone(),
    };
    let writer = ChannelWriter { sender };
    let mut builder = tar::Builder::new(writer);
    builder.mode(tar::HeaderMode::Deterministic);
    builder
        .append_data(&mut header, name, reader)
        .map_err(|error| error.to_string())?;
    builder.finish().map_err(|error| error.to_string())?;
    Ok(())
}

fn file_mode(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o644
    }
}

/// 目录上传的 tar 输出：写入通道并累计字节数上报进度。
pub(super) struct ProgressChannelWriter {
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
    written: u64,
    progress: flume::Sender<(u64, Option<u64>)>,
}

impl Write for ProgressChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender
            .send(Ok(buf.to_vec()))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "上传通道已关闭"))?;
        self.written = self.written.saturating_add(buf.len() as u64);
        let _ = self.progress.try_send((self.written, None));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 把 tar 输出写入有界通道；通道满时阻塞，形成反压。
pub(super) struct ChannelWriter {
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender
            .send(Ok(buf.to_vec()))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "上传通道已关闭"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 读取本地文件并上报进度；取消时返回错误终止 tar 生成。
pub(super) struct ProgressReader {
    inner: std::fs::File,
    read: u64,
    total: u64,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
}

impl Read for ProgressReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("上传已取消"));
        }
        let count = self.inner.read(buf)?;
        self.read = self.read.saturating_add(count as u64);
        let _ = self.progress.try_send((self.read, Some(self.total)));
        Ok(count)
    }
}
