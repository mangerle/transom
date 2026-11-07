//! 列目录、文件预览与下载。
use super::archive::{download_to_file, open_archive, write_archive_to_file};
use super::listing::{parse_listing, read_first_file};
use super::validate_container_path;
use crate::engine::Engine;
use crate::error::ContainerError;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use transom_core::ContainerFileEntry;

/// 列目录：解析归档 tar 的直接子项。
///
/// # Errors
/// 路径非法、容器/路径不存在或 tar 解析失败时返回 [`ContainerError`]。
pub async fn list_dir(
    engine: &Engine,
    container: &str,
    path: &str,
) -> Result<Vec<ContainerFileEntry>, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let result = tokio::task::spawn_blocking(move || parse_listing(reader, &display))
        .await
        .map_err(|error| ContainerError::File {
            path: path.clone(),
            message: format!("解析目录任务失败: {error}"),
        })?;
    task.abort();
    result
}

/// 读取文件预览；超过 `max_bytes` 时截断并标记。
///
/// # Errors
/// 路径非法、文件不存在、读取失败或 tar 解析失败时返回 [`ContainerError`]。
pub async fn read_file(
    engine: &Engine,
    container: &str,
    path: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool, u64), ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let result = tokio::task::spawn_blocking(move || read_first_file(reader, &display, max_bytes))
        .await
        .map_err(|error| ContainerError::File {
            path: path.clone(),
            message: format!("读取文件任务失败: {error}"),
        })?;
    task.abort();
    result
}

/// 下载文件到本地路径（先写临时文件，成功后原子改名）。
///
/// 进度通过有界通道上报，界面来不及消费时丢弃样本而不是阻塞传输。
///
/// # Errors
/// 路径非法、容器文件不存在、本地写盘失败或用户取消时返回 [`ContainerError`]。
pub async fn download(
    engine: &Engine,
    container: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    if let Some(parent) = local_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        return Err(ContainerError::File {
            path: local_path.display().to_string(),
            message: "本地目标目录不存在".to_string(),
        });
    }
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let destination = local_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        download_to_file(reader, &display, &destination, &cancel, &progress)
    })
    .await
    .map_err(|error| ContainerError::File {
        path: path.clone(),
        message: format!("下载任务失败: {error}"),
    })?;
    task.abort();
    result
}

/// 把容器内目录原样导出为本地 tar 归档。
///
/// 与单文件下载不同，这里不做 tar 解包，直接把归档 API 的原始 tar 流写入磁盘，
/// 因此目录层级、符号链接与权限都能保留。
///
/// # Errors
/// 路径非法、目标目录不存在、读取中断或用户取消时返回 [`ContainerError`]。
pub async fn download_directory(
    engine: &Engine,
    container: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    if let Some(parent) = local_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        return Err(ContainerError::File {
            path: local_path.display().to_string(),
            message: "本地目标目录不存在".to_string(),
        });
    }
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let destination = local_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        write_archive_to_file(reader, &display, &destination, &cancel, &progress)
    })
    .await
    .map_err(|error| ContainerError::File {
        path: path.clone(),
        message: format!("目录导出任务失败: {error}"),
    })?;
    task.abort();
    result
}
