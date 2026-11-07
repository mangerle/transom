//! 整卷 tar 备份与恢复。

use super::probe::TemporaryTargetGuard;
use super::{resolve_volume_target, temp_path};
use crate::engine::Engine;
use crate::error::ContainerError;
use crate::files::open_archive;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 备份数据卷为本地 Tar 归档文件。
///
/// 将数据卷挂载点根目录下的全部内容通过 Docker 归档流式传输直接保存为本地 Tar 文件，
/// 完整保留目录层级、文件元数据与权限。
pub async fn backup_volume(
    engine: &Engine,
    volume: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let target = resolve_volume_target(&api, volume, false).await?;
    let guard = TemporaryTargetGuard::new(&api, &target);
    let container_path = format!("{}/.", target.mount_path.trim_end_matches('/'));

    let (reader, task) = open_archive(&api, &target.container_id, &container_path).await?;
    let destination = local_path.to_path_buf();
    let cancel_clone = Arc::clone(&cancel);

    let result = tokio::task::spawn_blocking(move || {
        let temp = temp_path(&destination);
        let mut file = std::fs::File::create(&temp).map_err(|e| ContainerError::File {
            path: temp.display().to_string(),
            message: e.to_string(),
        })?;
        let mut reader = reader;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut written = 0u64;

        loop {
            if cancel_clone.load(Ordering::Relaxed) {
                let _ = std::fs::remove_file(&temp);
                return Err(ContainerError::Cancelled);
            }
            use std::io::Read;
            let n = reader.read(&mut buffer).map_err(|e| ContainerError::File {
                path: temp.display().to_string(),
                message: e.to_string(),
            })?;
            if n == 0 {
                break;
            }
            use std::io::Write;
            file.write_all(&buffer[..n])
                .map_err(|e| ContainerError::File {
                    path: temp.display().to_string(),
                    message: e.to_string(),
                })?;
            written = written.saturating_add(n as u64);
            let _ = progress.try_send((written, None));
        }

        std::fs::rename(&temp, &destination).map_err(|e| ContainerError::File {
            path: destination.display().to_string(),
            message: e.to_string(),
        })?;
        Ok(written)
    })
    .await
    .map_err(|e| ContainerError::File {
        path: local_path.display().to_string(),
        message: e.to_string(),
    })?;

    task.abort();
    guard.dismiss().await;
    result
}

/// 判断“创建数据卷”失败是否只是卷已存在。
pub(super) fn is_volume_already_exists(error: &ContainerError) -> bool {
    match error {
        ContainerError::Api { status, body, .. } => {
            *status == 409 || body.to_ascii_lowercase().contains("already exists")
        }
        ContainerError::Cli { message, .. } => {
            message.to_ascii_lowercase().contains("already exists")
        }
        _ => false,
    }
}

/// 从本地 Tar 归档文件恢复到指定数据卷中。
///
/// 若数据卷尚不存在，将先创建新卷；然后将本地 Tar 归档直接通过 Docker 归档 API
/// 流式写入卷中解压，完成数据卷冷备份还原。
pub async fn restore_volume(
    engine: &Engine,
    volume: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    // 1. 若目标数据卷不存在则先创建；已存在不是错误，但权限/引擎故障必须回传。
    if let Err(error) = engine.create_volume(volume, None).await
        && !is_volume_already_exists(&error)
    {
        return Err(error);
    }

    // 2. 解析挂载目标（探针或已有容器），指定可写模式
    let target = resolve_volume_target(&api, volume, true).await?;
    let guard = TemporaryTargetGuard::new(&api, &target);
    let container_path = target.mount_path.clone();

    // 3. 读取本地 tar 文件并通过 PUT archive 流式写入容器
    let metadata = std::fs::metadata(local_path).map_err(|e| ContainerError::File {
        path: local_path.display().to_string(),
        message: e.to_string(),
    })?;
    let total = metadata.len();

    let (sender, receiver) = flume::bounded::<Result<Vec<u8>, std::io::Error>>(8);
    let builder_cancel = Arc::clone(&cancel);
    let builder_progress = progress.clone();
    let builder_local = local_path.to_path_buf();

    let reader_task = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&builder_local)?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut written = 0u64;

        loop {
            if builder_cancel.load(Ordering::Relaxed) {
                break;
            }
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            written = written.saturating_add(n as u64);
            let _ = builder_progress.try_send((written, Some(total)));
            if sender.send(Ok(buffer[..n].to_vec())).is_err() {
                break;
            }
        }
        Ok::<(), std::io::Error>(())
    });

    let stream = futures::stream::unfold(receiver, |receiver| async move {
        receiver
            .recv_async()
            .await
            .ok()
            .map(|item| (item, receiver))
    });
    let body = StreamBody::new(stream.map(|item| match item {
        Ok(bytes) => Ok(Frame::data(Bytes::from(bytes))),
        Err(e) => Err(e),
    }));

    let suffix = format!(
        "/containers/{}/archive?path={}",
        crate::http::percent_encode(&target.container_id),
        crate::http::percent_encode(&container_path)
    );
    let response = crate::http::request_stream(
        api.transport_ref(),
        hyper::Method::PUT,
        &api.versioned_path(&suffix),
        body,
        Some("application/x-tar"),
    )
    .await;

    let reader_result = reader_task.await.map_err(|error| ContainerError::File {
        path: local_path.display().to_string(),
        message: format!("读取恢复归档任务失败: {error}"),
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    guard.dismiss().await;

    let response = response?;
    reader_result.map_err(|error| ContainerError::File {
        path: local_path.display().to_string(),
        message: format!("读取恢复归档失败: {error}"),
    })?;
    if !response.status.is_success() {
        let status = response.status;
        let body = crate::http::read_body_text(response.body, 8192).await;
        crate::http::ensure_success(status, "恢复数据卷", body, Some(volume))?;
    }

    Ok(total)
}
