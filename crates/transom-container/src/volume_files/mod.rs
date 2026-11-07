//! 数据卷文件浏览与提取：复用容器归档 API 实现卷内文件访问。
//!
//! 设计原理（Why）：
//! - Docker 原生没有 `/volumes/{name}/archive` 接口，无法直接从卷读取文件；
//! - 若有容器挂载了该卷，可直接通过该容器的挂载点映射复用 Docker 归档 API；
//! - 若无任何容器挂载（孤立卷），创建轻量只读探针容器（无需运行，仅 created
//!   状态即可被归档 API 读取），读取完成后立即销毁该探针容器；
//! - 这种方案全平台（Linux、macOS、Windows、WSL、SSH 远端）行为完全一致，
//!   无需宿主机 root 权限或直接接触卷在宿主机上的存储目录。
//!
//! probe.rs 负责挂载容器/探针容器解析，backup.rs 负责整卷备份与恢复。

mod backup;
mod probe;
#[cfg(test)]
mod tests;

pub use backup::{backup_volume, restore_volume};

use crate::engine::Engine;
use crate::error::ContainerError;
use crate::files::{download, list_dir, read_file, validate_container_path, write_file_bytes};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use transom_core::ContainerFileEntry;

use probe::{TemporaryTargetGuard, resolve_volume_target};

/// 将卷内相对路径转换为目标容器内的绝对路径。
fn map_volume_path(mount_path: &str, path: &str) -> Result<String, ContainerError> {
    let validated = validate_container_path(path)?;
    if validated == "/" {
        Ok(mount_path.to_string())
    } else {
        let trimmed_mount = mount_path.trim_end_matches('/');
        let trimmed_path = validated.trim_start_matches('/');
        Ok(format!("{trimmed_mount}/{trimmed_path}"))
    }
}

/// 将容器内绝对路径映射回卷内相对路径。
fn unmap_volume_path(mount_path: &str, container_path: &str) -> String {
    let trimmed_mount = mount_path.trim_end_matches('/');
    if container_path == trimmed_mount || container_path.is_empty() {
        return "/".to_string();
    }
    if let Some(stripped) = container_path.strip_prefix(trimmed_mount) {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            format!("/{stripped}")
        }
    } else {
        container_path.to_string()
    }
}

pub(super) fn temp_path(destination: &Path) -> PathBuf {
    let mut file_name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("volume-archive"));
    file_name.push(".transom-part");
    destination.with_file_name(file_name)
}

/// 列出数据卷内指定路径的文件与子目录。
///
/// # Errors
/// 引擎错误、路径非法或解析失败时返回 [`ContainerError`]。
pub async fn list_volume_dir(
    engine: &Engine,
    volume: &str,
    path: &str,
) -> Result<Vec<ContainerFileEntry>, ContainerError> {
    let api = engine.http()?.clone();
    let target = resolve_volume_target(&api, volume, false).await?;
    let guard = TemporaryTargetGuard::new(&api, &target);
    let container_path = map_volume_path(&target.mount_path, path);
    let result = match container_path {
        Ok(c_path) => list_dir(engine, &target.container_id, &c_path).await,
        Err(e) => Err(e),
    };
    guard.dismiss().await;
    let mut entries = result?;
    for entry in &mut entries {
        entry.path = unmap_volume_path(&target.mount_path, &entry.path);
    }
    Ok(entries)
}

/// 读取数据卷内指定文件的文本内容进行预览。
///
/// # Errors
/// 引擎错误、路径非法或解析失败时返回 [`ContainerError`]。
pub async fn read_volume_file(
    engine: &Engine,
    volume: &str,
    path: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool, u64), ContainerError> {
    let api = engine.http()?.clone();
    let target = resolve_volume_target(&api, volume, false).await?;
    let guard = TemporaryTargetGuard::new(&api, &target);
    let container_path = map_volume_path(&target.mount_path, path);
    let result = match container_path {
        Ok(c_path) => read_file(engine, &target.container_id, &c_path, max_bytes).await,
        Err(e) => Err(e),
    };
    guard.dismiss().await;
    result
}

/// 从数据卷中下载文件到本地路径。
///
/// # Errors
/// 引擎错误、路径非法或本地写入失败时返回 [`ContainerError`]。
pub async fn download_volume_file(
    engine: &Engine,
    volume: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let target = resolve_volume_target(&api, volume, false).await?;
    let guard = TemporaryTargetGuard::new(&api, &target);
    let container_path = map_volume_path(&target.mount_path, path);
    let result = match container_path {
        Ok(c_path) => {
            download(
                engine,
                &target.container_id,
                &c_path,
                local_path,
                cancel,
                progress,
            )
            .await
        }
        Err(e) => Err(e),
    };
    guard.dismiss().await;
    result
}

/// 向数据卷内指定路径直接写入文件字节数据。
///
/// # Errors
/// 路径非法、无可用挂载容器且创建探针容器失败、或归档写入失败时返回 [`ContainerError`]。
pub async fn write_volume_file_bytes(
    engine: &Engine,
    volume: &str,
    path: &str,
    data: Vec<u8>,
) -> Result<(), ContainerError> {
    let api = engine.http()?.clone();
    let target = resolve_volume_target(&api, volume, true).await?;
    let guard = TemporaryTargetGuard::new(&api, &target);
    let container_path = map_volume_path(&target.mount_path, path);
    let result = match container_path {
        Ok(c_path) => write_file_bytes(engine, &target.container_id, &c_path, data).await,
        Err(e) => Err(e),
    };
    guard.dismiss().await;
    result
}
