//! 镜像归档导入与导出（Save / Load）实现模块。
//!
//! 设计原理（Why）：
//! - 镜像导出（Save）：
//!   Docker / Podman REST 规范通过 `GET /images/{name}/get` 导出完整的镜像 Tarball 归档流；
//!   直接流式边接收边写入本地临时文件，并在分块传输中精确上报进度，避免大镜像把全部数据缓存在内存中导致 OOM；
//!   CLI 兜底调用 `docker save <image> -o <destination>`；
//! - 镜像导入（Load）：
//!   通过 `POST /images/load` 流式上传本地 Tarball 归档文件并由引擎载入解包；
//!   CLI 兜底调用 `docker load -i <source>`。

use crate::api::DockerApi;
use crate::bridge::{BlockingReader, output_channel};
use crate::cli::{CLI_ARCHIVE_TIMEOUT, CliBackend};
use crate::engine::Engine;
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use hyper::Method;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 临时文件路径生成器，确保写入过程原子性与安全落盘。
fn temp_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "image-archive".to_string());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    destination.with_file_name(format!(".{name}.{stamp}.transom-part"))
}

/// 基于 REST API 导出镜像为 Tar 归档文件。
async fn save_image_api(
    api: &DockerApi,
    image: &str,
    destination: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        return Err(ContainerError::File {
            path: destination.display().to_string(),
            message: "本地目标目录不存在".to_string(),
        });
    }

    let path = format!("/images/{}/get", http::percent_encode(image));
    let response = http::request(
        api.transport_ref(),
        Method::GET,
        &api.versioned_path(&path),
        Bytes::new(),
        None,
    )
    .await?;

    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        if status == hyper::StatusCode::NOT_FOUND {
            return Err(ContainerError::NotFound {
                target: image.to_string(),
            });
        }
        return Err(ContainerError::Api {
            operation: format!("导出镜像 {image}"),
            status: status.as_u16(),
            body,
        });
    }

    let (sender, reader) = output_channel(8);
    let task = http::pump_body(response, sender);
    let temp = temp_path(destination);
    let destination_buf = destination.to_path_buf();
    let temp_buf = temp.clone();

    let result = tokio::task::spawn_blocking(move || {
        write_archive_stream(reader, &temp_buf, &destination_buf, &cancel, &progress)
    })
    .await
    .map_err(|error| ContainerError::File {
        path: destination.display().to_string(),
        message: format!("镜像导出写入任务失败: {error}"),
    })?;

    task.abort();
    result
}

/// 阻塞流式写入归档文件并汇报传输进度。
fn write_archive_stream(
    mut reader: BlockingReader,
    temp: &Path,
    destination: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let mut file = std::fs::File::create(temp).map_err(|error| ContainerError::File {
        path: temp.display().to_string(),
        message: error.to_string(),
    })?;

    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(temp);
            return Err(ContainerError::Cancelled);
        }

        let count = reader
            .read(&mut buffer)
            .map_err(|error| ContainerError::File {
                path: temp.display().to_string(),
                message: format!("读取响应流失败: {error}"),
            })?;

        if count == 0 {
            break;
        }

        file.write_all(&buffer[..count])
            .map_err(|error| ContainerError::File {
                path: temp.display().to_string(),
                message: format!("写入文件失败: {error}"),
            })?;

        written = written.saturating_add(count as u64);
        let _ = progress.send((written, None));
    }

    file.flush().map_err(|error| ContainerError::File {
        path: temp.display().to_string(),
        message: format!("刷入磁盘失败: {error}"),
    })?;
    drop(file);

    std::fs::rename(temp, destination).map_err(|error| ContainerError::File {
        path: destination.display().to_string(),
        message: format!("重命名归档文件失败: {error}"),
    })?;

    Ok(written)
}

/// 基于 Docker CLI 导出镜像为 Tar 归档文件。
async fn save_image_cli(
    cli: &CliBackend,
    image: &str,
    destination: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }

    let temp = temp_path(destination);
    let temp_str = temp.display().to_string();

    let result = cli
        .run_checked_cancellable(
            &[
                "save".to_string(),
                image.to_string(),
                "-o".to_string(),
                temp_str,
            ],
            CLI_ARCHIVE_TIMEOUT,
            cancel,
        )
        .await;

    if cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&temp);
        return Err(ContainerError::Cancelled);
    }
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }

    result?;

    let bytes = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
    std::fs::rename(&temp, destination).map_err(|error| ContainerError::File {
        path: destination.display().to_string(),
        message: format!("重命名归档文件失败: {error}"),
    })?;

    let _ = progress.send((bytes, Some(bytes)));
    Ok(bytes)
}

/// 导出指定镜像为本地 Tar 归档文件（Save）。
///
/// # Errors
/// 目标目录不存在、网络传输中断、磁盘写入失败或用户取消时返回 [`ContainerError`]。
pub async fn save_image(
    engine: &Engine,
    image: &str,
    destination: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    if let Ok(api) = engine.http() {
        save_image_api(api, image, destination, cancel, progress).await
    } else if let Some(cli) = engine.cli_backend_ref() {
        save_image_cli(cli, image, destination, &cancel, &progress).await
    } else {
        Err(ContainerError::Unsupported {
            operation: "镜像导出 (Save)".to_string(),
        })
    }
}

/// 基于 REST API 导入本地 Tar 归档镜像包（Load）。
async fn load_image_api(
    api: &DockerApi,
    source: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let metadata = std::fs::metadata(source).map_err(|error| ContainerError::File {
        path: source.display().to_string(),
        message: format!("无法读取本地镜像文件: {error}"),
    })?;
    if !metadata.is_file() {
        return Err(ContainerError::File {
            path: source.display().to_string(),
            message: "指定的镜像归档必须是普通文件".to_string(),
        });
    }
    let total = metadata.len();
    let (sender, receiver) = flume::bounded::<Result<Vec<u8>, std::io::Error>>(8);
    let sender_cancel = Arc::clone(&cancel);
    let sender_progress = progress.clone();
    let source_path = source.to_path_buf();

    let reader_task = tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&source_path)?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut transferred = 0u64;
        loop {
            if sender_cancel.load(Ordering::Relaxed) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "操作已取消",
                ));
            }
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            transferred = transferred.saturating_add(count as u64);
            let _ = sender_progress.send((transferred, Some(total)));
            if sender.send(Ok(buffer[..count].to_vec())).is_err() {
                break;
            }
        }
        Ok(())
    });

    use futures::StreamExt;
    use http_body_util::StreamBody;
    use hyper::body::Frame;

    let stream = futures::stream::unfold(receiver, |receiver| async move {
        receiver
            .recv_async()
            .await
            .ok()
            .map(|item| (item, receiver))
    });
    let body = StreamBody::new(stream.map(|item| match item {
        Ok(bytes) => Ok(Frame::data(Bytes::from(bytes))),
        Err(error) => Err(error),
    }));

    let response = http::request_stream(
        api.transport_ref(),
        Method::POST,
        &api.versioned_path("/images/load?quiet=false"),
        body,
        Some("application/x-tar"),
    )
    .await;

    let reader_result = reader_task.await.map_err(|error| ContainerError::File {
        path: source.display().to_string(),
        message: format!("镜像读取线程失败: {error}"),
    })?;

    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    if let Err(err) = reader_result {
        if err.kind() == std::io::ErrorKind::Interrupted {
            return Err(ContainerError::Cancelled);
        }
        return Err(ContainerError::File {
            path: source.display().to_string(),
            message: format!("读取镜像文件失败: {err}"),
        });
    }

    let response = response?;
    let body_text = http::read_body_text(response.body, 64 * 1024).await;
    if !response.status.is_success() {
        return Err(ContainerError::Api {
            operation: "导入镜像 (Load)".to_string(),
            status: response.status.as_u16(),
            body: body_text,
        });
    }

    // 检查响应 JSON 行中是否存在错误信息
    for line in body_text.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(err) = val.get("error").and_then(|v| v.as_str())
        {
            return Err(ContainerError::Api {
                operation: "导入镜像 (Load)".to_string(),
                status: 400,
                body: err.to_string(),
            });
        }
    }

    Ok(total)
}

/// 基于 Docker CLI 导入本地 Tar 归档镜像包（Load）。
async fn load_image_cli(
    cli: &CliBackend,
    source: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }

    let total = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    let _ = progress.send((0, Some(total)));

    let source_str = source.display().to_string();
    let result = cli
        .run_checked_cancellable(
            &["load".to_string(), "-i".to_string(), source_str],
            CLI_ARCHIVE_TIMEOUT,
            cancel,
        )
        .await;

    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }

    result?;
    let _ = progress.send((total, Some(total)));
    Ok(total)
}

/// 从本地 Tar 归档文件导入镜像（Load）。
///
/// # Errors
/// 本地文件不存在、网络上传中断、Docker 载入失败或用户取消时返回 [`ContainerError`]。
pub async fn load_image(
    engine: &Engine,
    source: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    if let Ok(api) = engine.http() {
        load_image_api(api, source, cancel, progress).await
    } else if let Some(cli) = engine.cli_backend_ref() {
        load_image_cli(cli, source, &cancel, &progress).await
    } else {
        Err(ContainerError::Unsupported {
            operation: "镜像导入 (Load)".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_path_generation() {
        let dest = Path::new("/tmp/images/ubuntu_latest.tar");
        let temp = temp_path(dest);
        let temp_name = temp.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(temp_name.starts_with(".ubuntu_latest.tar."));
        assert!(temp_name.ends_with(".transom-part"));
        assert_eq!(temp.parent(), dest.parent());
    }
}
