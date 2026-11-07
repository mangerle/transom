//! 卷文件访问目标解析：已挂载容器或临时只读探针容器。

use crate::api::DockerApi;
use crate::error::ContainerError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use transom_core::{ContainerCreateSpec, ContainerLifecycleAction};

static PROBE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 卷文件访问目标：由已挂载容器或临时探针容器提供。
pub(super) struct VolumeTarget {
    pub(super) container_id: String,
    pub(super) mount_path: String,
    pub(super) is_temporary: bool,
}

/// 自动销毁临时探针容器的守卫对象。
pub(super) struct TemporaryTargetGuard<'a> {
    api: &'a DockerApi,
    container_id: Option<String>,
}

impl<'a> TemporaryTargetGuard<'a> {
    pub(super) fn new(api: &'a DockerApi, target: &VolumeTarget) -> Self {
        Self {
            api,
            container_id: if target.is_temporary {
                Some(target.container_id.clone())
            } else {
                None
            },
        }
    }

    pub(super) async fn dismiss(mut self) {
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
pub(super) async fn resolve_volume_target(
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
pub(super) fn select_probe_image(images: &[transom_core::ContainerImageSummary]) -> Option<String> {
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
