//! 数据卷文件浏览与提取：复用容器归档 API 实现卷内文件访问。
//!
//! 设计原理（Why）：
//! - Docker 原生没有 `/volumes/{name}/archive` 接口，无法直接从卷读取文件；
//! - 若有容器挂载了该卷，可直接通过该容器的挂载点映射复用 Docker 归档 API；
//! - 若无任何容器挂载（孤立卷），创建轻量只读探针容器（无需运行，仅 created 状态即可
//!   被归档 API 读取），读取完成后立即销毁该探针容器；
//! - 这种方案全平台（Linux、macOS、Windows、WSL、SSH 远端）行为完全一致，
//!   无需宿主机 root 权限或直接接触卷在宿主机上的存储目录。

use crate::api::DockerApi;
use crate::engine::Engine;
use crate::error::ContainerError;
use crate::files::{download, list_dir, read_file, validate_container_path};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;
use transom_core::{
    ContainerCreateSpec, ContainerFileEntry, ContainerLifecycleAction,
};

static PROBE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 卷文件访问目标：由已挂载容器或临时探针容器提供。
struct VolumeTarget {
    container_id: String,
    mount_path: String,
    is_temporary: bool,
}

/// 解析或创建用于访问指定卷的目标容器与挂载点。
async fn resolve_volume_target(
    api: &DockerApi,
    volume: &str,
) -> Result<VolumeTarget, ContainerError> {
    // 1. 优先在现有容器中查找是否已有挂载该卷的容器
    if let Ok(containers) = api.list_containers().await {
        for summary in containers {
            for mount in summary.mounts {
                if mount.source == volume || mount.source.ends_with(&format!("/{volume}")) {
                    return Ok(VolumeTarget {
                        container_id: summary.id,
                        mount_path: mount.destination,
                        is_temporary: false,
                    });
                }
            }
        }
    }

    // 2. 无现有容器挂载该卷，创建极简只读探针容器
    let images = api.list_images().await.unwrap_or_default();
    let image_name = images
        .iter()
        .find_map(|img| img.tags.first().cloned())
        .or_else(|| images.first().map(|img| img.id.clone()))
        .unwrap_or_else(|| "alpine".to_string());

    let count = PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let probe_name = format!("transom-vol-{now}-{count}");

    let spec = ContainerCreateSpec {
        image: image_name,
        name: probe_name,
        binds: vec![format!("{volume}:/transom_vol:ro")],
        ..Default::default()
    };
    let container_id = api.create_container(&spec).await?;

    Ok(VolumeTarget {
        container_id,
        mount_path: "/transom_vol".to_string(),
        is_temporary: true,
    })
}

/// 清理临时探针容器。
async fn cleanup_temporary(api: &DockerApi, container_id: &str) {
    let _ = api
        .lifecycle(
            container_id,
            ContainerLifecycleAction::Remove { force: true },
        )
        .await;
}

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
    let target = resolve_volume_target(&api, volume).await?;
    let container_path = map_volume_path(&target.mount_path, path);
    let result = match container_path {
        Ok(c_path) => list_dir(engine, &target.container_id, &c_path).await,
        Err(e) => Err(e),
    };
    if target.is_temporary {
        cleanup_temporary(&api, &target.container_id).await;
    }
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
    let target = resolve_volume_target(&api, volume).await?;
    let container_path = map_volume_path(&target.mount_path, path);
    let result = match container_path {
        Ok(c_path) => read_file(engine, &target.container_id, &c_path, max_bytes).await,
        Err(e) => Err(e),
    };
    if target.is_temporary {
        cleanup_temporary(&api, &target.container_id).await;
    }
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
    let target = resolve_volume_target(&api, volume).await?;
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
    if target.is_temporary {
        cleanup_temporary(&api, &target.container_id).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{map_volume_path, unmap_volume_path};

    #[test]
    fn test_volume_path_mapping() {
        let mount = "/transom_vol";
        assert_eq!(map_volume_path(mount, "/").unwrap(), "/transom_vol");
        assert_eq!(
            map_volume_path(mount, "/sub/dir").unwrap(),
            "/transom_vol/sub/dir"
        );
        assert_eq!(
            map_volume_path("/mnt/data/", "/file.txt").unwrap(),
            "/mnt/data/file.txt"
        );

        assert_eq!(unmap_volume_path(mount, "/transom_vol"), "/");
        assert_eq!(
            unmap_volume_path(mount, "/transom_vol/sub/dir"),
            "/sub/dir"
        );
        assert_eq!(
            unmap_volume_path("/mnt/data", "/mnt/data/file.txt"),
            "/file.txt"
        );
    }
}
