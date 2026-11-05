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
use crate::files::{
    download, list_dir, open_archive, read_file, validate_container_path, write_file_bytes,
};
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;
use transom_core::{ContainerCreateSpec, ContainerFileEntry, ContainerLifecycleAction};

static PROBE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 卷文件访问目标：由已挂载容器或临时探针容器提供。
struct VolumeTarget {
    container_id: String,
    mount_path: String,
    is_temporary: bool,
}

/// 自动销毁临时探针容器的守卫对象。
struct TemporaryTargetGuard<'a> {
    api: &'a DockerApi,
    container_id: Option<String>,
}

impl<'a> TemporaryTargetGuard<'a> {
    fn new(api: &'a DockerApi, target: &VolumeTarget) -> Self {
        Self {
            api,
            container_id: if target.is_temporary {
                Some(target.container_id.clone())
            } else {
                None
            },
        }
    }

    async fn dismiss(mut self) {
        if let Some(id) = self.container_id.take() {
            cleanup_temporary(self.api, &id).await;
        }
    }
}

impl Drop for TemporaryTargetGuard<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.container_id.take() {
            let api = self.api.clone();
            tokio::spawn(async move {
                cleanup_temporary(&api, &id).await;
            });
        }
    }
}

/// 解析或创建用于访问指定卷的目标容器与挂载点。
///
/// # 设计原理（Why）
/// - 若存在已有容器挂载了该卷，优先复用该容器，避免容器创建销毁开销；
/// - 若为写操作（`writeable = true`），校验挂载点必须为可写（`rw`），避免向只读容器写文件触发引擎报错；
/// - 若无可用容器，创建轻量临时探针容器。挂载模式严格根据读写需求指定为 `:rw` 或 `:ro`；
/// - 镜像选择上优先匹配 alpine/busybox/debian 等小型 Linux 发行版；若本地无可用镜像，尝试自动拉取极简镜像；
/// - 利用 [`TemporaryTargetGuard`] 保障探针容器在退出时自动销毁，避免遗留孤儿容器。
async fn resolve_volume_target(
    api: &DockerApi,
    volume: &str,
    writeable: bool,
) -> Result<VolumeTarget, ContainerError> {
    // 1. 优先在现有容器中查找是否已有挂载该卷且满足读写要求的容器
    if let Ok(containers) = api.list_containers().await {
        for summary in containers {
            for mount in summary.mounts {
                // 按卷名匹配：命名卷的 Source 是宿主挂载点，不能用来识别卷。
                if mount.volume_name() == Some(volume) {
                    // 若需要写入，则挂载点必须为可写（rw 为 true，且 mode 不包含 ro）
                    if writeable && (!mount.rw || mount.mode.contains("ro")) {
                        continue;
                    }
                    return Ok(VolumeTarget {
                        container_id: summary.id,
                        mount_path: mount.destination,
                        is_temporary: false,
                    });
                }
            }
        }
    }

    // 2. 无现有匹配容器，智能选择本地可用镜像创建探针容器
    let images = api.list_images().await.unwrap_or_default();
    let selected_image = select_probe_image(&images);
    let image_name = match selected_image {
        Some(name) => name,
        None => {
            log::info!("本地未发现可用镜像，尝试自动拉取 alpine 探针镜像以访问数据卷: {volume}");
            let mut pull_error = None;
            let reference = "alpine:latest";
            let mut dummy = |_, _| {};
            if let Err(err) = api.pull_image(reference, &mut dummy).await {
                pull_error = Some(err);
            }
            if let Some(err) = pull_error {
                return Err(ContainerError::NoEngine {
                    detail: format!(
                        "访问数据卷需要临时探针容器，但本地无可用镜像且自动拉取 alpine 失败: {err}"
                    ),
                });
            }
            reference.to_string()
        }
    };

    let count = PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let probe_name = format!("transom-vol-{now}-{count}");
    let mode = if writeable { "rw" } else { "ro" };

    let spec = ContainerCreateSpec {
        image: image_name,
        name: probe_name,
        binds: vec![format!("{volume}:/transom_vol:{mode}")],
        ..Default::default()
    };
    let container_id = api.create_container(&spec).await?;

    Ok(VolumeTarget {
        container_id,
        mount_path: "/transom_vol".to_string(),
        is_temporary: true,
    })
}

/// 优先选择轻量级系统镜像作为探针容器镜像。
fn select_probe_image(images: &[transom_core::ContainerImageSummary]) -> Option<String> {
    const PREFERRED_KEYWORDS: &[&str] = &[
        "alpine",
        "busybox",
        "debian",
        "ubuntu",
        "distroless",
        "archlinux",
    ];
    // 第一优先级：匹配常见系统镜像关键字
    for keyword in PREFERRED_KEYWORDS {
        for img in images {
            for tag in &img.tags {
                if tag.contains(keyword) && !tag.contains("<none>") {
                    return Some(tag.clone());
                }
            }
        }
    }
    // 第二优先级：带有正常命名 tag 的任意可用镜像
    for img in images {
        if let Some(tag) = img.tags.iter().find(|t| !t.contains("<none>")) {
            return Some(tag.clone());
        }
    }
    // 第三优先级：镜像 ID
    images.first().map(|img| img.id.clone())
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

fn temp_path(destination: &Path) -> PathBuf {
    let mut file_name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("volume-archive"));
    file_name.push(".transom-part");
    destination.with_file_name(file_name)
}

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
fn is_volume_already_exists(error: &ContainerError) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{is_volume_already_exists, map_volume_path, unmap_volume_path};
    use crate::error::ContainerError;

    #[test]
    fn volume_create_conflict_is_tolerated_but_real_failure_is_not() {
        assert!(is_volume_already_exists(&ContainerError::Api {
            operation: "创建数据卷".into(),
            status: 409,
            body: "volume already exists".into(),
        }));
        assert!(!is_volume_already_exists(&ContainerError::Api {
            operation: "创建数据卷".into(),
            status: 500,
            body: "permission denied".into(),
        }));
        assert!(is_volume_already_exists(&ContainerError::Cli {
            command: "docker volume create".into(),
            message: "volume already exists".into(),
        }));
    }

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
        assert_eq!(unmap_volume_path(mount, "/transom_vol/sub/dir"), "/sub/dir");
        assert_eq!(
            unmap_volume_path("/mnt/data", "/mnt/data/file.txt"),
            "/file.txt"
        );
    }

    #[test]
    fn test_volume_temp_path() {
        use super::temp_path;
        use std::path::Path;
        let dest = Path::new("/tmp/my_vol.tar");
        let temp = temp_path(dest);
        assert_eq!(temp, Path::new("/tmp/my_vol.tar.transom-part"));
    }

    #[test]
    fn test_select_probe_image() {
        use super::select_probe_image;
        use transom_core::ContainerImageSummary;

        let empty: Vec<ContainerImageSummary> = Vec::new();
        assert_eq!(select_probe_image(&empty), None);

        let regular_img = ContainerImageSummary {
            id: "sha256:111".to_string(),
            tags: vec!["my-custom-service:v1".to_string()],
            size: 1024,
            created: 0,
        };
        let alpine_img = ContainerImageSummary {
            id: "sha256:222".to_string(),
            tags: vec!["docker.io/library/alpine:3.18".to_string()],
            size: 512,
            created: 0,
        };

        // 优先匹配 alpine 关键字
        let images = vec![regular_img.clone(), alpine_img.clone()];
        assert_eq!(
            select_probe_image(&images),
            Some("docker.io/library/alpine:3.18".to_string())
        );

        // 无关键字时回落到有效 tag 镜像
        let images_no_preferred = vec![regular_img.clone()];
        assert_eq!(
            select_probe_image(&images_no_preferred),
            Some("my-custom-service:v1".to_string())
        );
    }
}
