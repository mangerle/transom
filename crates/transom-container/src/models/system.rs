//! 引擎级响应：版本信息、系统磁盘占用与事件流。

use super::non_blank;
use serde::Deserialize;

/// `GET /version` 响应（只取需要的字段）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VersionResponse {
    #[serde(rename = "Version", default)]
    pub version: Option<String>,
    #[serde(rename = "ApiVersion", default)]
    pub api_version: Option<String>,
    #[serde(rename = "Platform", default)]
    pub platform: Option<PlatformInfo>,
    #[serde(rename = "Components", default)]
    pub components: Option<Vec<ComponentInfo>>,
    /// 引擎运行系统；`Platform.Name` 为空时用于兜底展示。
    #[serde(rename = "Os", default)]
    pub os: Option<String>,
    /// 引擎架构；与 `Os` 一起构成兜底平台名。
    #[serde(rename = "Arch", default)]
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlatformInfo {
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ComponentInfo {
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
}

impl VersionResponse {
    /// 识别是否为 Podman；识别不出时沿用调用方声明的引擎种类。
    pub fn is_podman(&self) -> bool {
        let platform = self
            .platform
            .as_ref()
            .and_then(|platform| platform.name.as_deref())
            .unwrap_or_default();
        if platform.to_ascii_lowercase().contains("podman") {
            return true;
        }
        self.components
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|component| component.name.as_deref())
            .any(|name| name.to_ascii_lowercase().contains("podman"))
    }

    /// 界面展示的平台名。
    ///
    /// 兜底次序（Why）：真机 Docker 29 的 `/version` 把 `Platform.Name` 返回成
    /// 空串（键存在但无内容），直接展示会得到一片空白；因此先取非空的平台名，
    /// 再回落到 `os/arch`，最后才是通用文案。
    pub fn platform_name(&self) -> String {
        non_blank(
            self.platform
                .as_ref()
                .and_then(|platform| platform.name.as_deref()),
        )
        .unwrap_or_else(|| {
            match (
                non_blank(self.os.as_deref()),
                non_blank(self.arch.as_deref()),
            ) {
                (Some(os), Some(arch)) => format!("{os}/{arch}"),
                (Some(os), None) | (None, Some(os)) => os,
                (None, None) => "Docker/Podman".to_string(),
            }
        })
    }
}

/// 系统全局磁盘占用响应（GET /system/df）。
///
/// 只声明聚合用量真正读取的字段；引擎新增的其余字段由 serde 直接忽略，
/// 避免为了“字段齐全”保留永远不读的占位字段。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SystemDfResponse {
    #[serde(default)]
    pub images: Option<Vec<DfImageItem>>,
    #[serde(default)]
    pub containers: Option<Vec<DfContainerItem>>,
    #[serde(default)]
    pub volumes: Option<Vec<DfVolumeItem>>,
    #[serde(default)]
    pub build_cache: Option<Vec<DfBuildCacheItem>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DfImageItem {
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub containers: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DfContainerItem {
    #[serde(default)]
    pub size_rw: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DfVolumeItem {
    #[serde(default)]
    pub usage_data: Option<DfVolumeUsageData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DfVolumeUsageData {
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub ref_count: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DfBuildCacheItem {
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub in_use: bool,
}

/// Docker 原生流式事件消息（GET /events）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DockerEventMessage {
    #[serde(alias = "Type", alias = "type", default)]
    pub resource_type: String,
    #[serde(alias = "Action", alias = "action", default)]
    pub action: String,
    #[serde(alias = "Actor", alias = "actor", default)]
    pub actor: Option<DockerEventActor>,
    #[serde(alias = "time", alias = "Time", default)]
    pub time: i64,
}

/// Docker 原生流式事件主体。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DockerEventActor {
    #[serde(rename = "ID", default)]
    pub id: String,
    #[serde(default)]
    pub attributes: Option<std::collections::HashMap<String, String>>,
}

impl DockerEventMessage {
    /// 转换为 transom-core 的标准 EngineEvent 模型。
    pub fn into_engine_event(self) -> Option<transom_core::EngineEvent> {
        let actor = self.actor?;
        let attributes = actor.attributes.unwrap_or_default();
        let name = attributes.get("name").cloned();
        let image = attributes.get("image").cloned();
        let exit_code = attributes
            .get("exitCode")
            .and_then(|s| s.parse::<i32>().ok());

        Some(transom_core::EngineEvent {
            resource_type: self.resource_type,
            action: self.action,
            id: actor.id,
            name,
            image,
            exit_code,
            timestamp: self.time,
        })
    }
}

impl SystemDfResponse {
    /// 转换为 transom-core 的聚合用量结构体。
    pub fn into_disk_usage(self) -> transom_core::ContainerSystemDiskUsage {
        let mut usage = transom_core::ContainerSystemDiskUsage::default();

        if let Some(images) = self.images {
            usage.images_count = images.len();
            for img in images {
                usage.images_size = usage.images_size.saturating_add(img.size);
                if img.containers <= 0 {
                    usage.images_reclaimable = usage.images_reclaimable.saturating_add(img.size);
                }
            }
        }

        if let Some(containers) = self.containers {
            usage.containers_count = containers.len();
            for c in containers {
                usage.containers_size = usage.containers_size.saturating_add(c.size_rw);
                // 容器的可写层空间一般在清理容器时释放
                usage.containers_reclaimable =
                    usage.containers_reclaimable.saturating_add(c.size_rw);
            }
        }

        if let Some(volumes) = self.volumes {
            usage.volumes_count = volumes.len();
            for v in volumes {
                if let Some(data) = v.usage_data {
                    let sz = data.size.max(0) as u64;
                    usage.volumes_size = usage.volumes_size.saturating_add(sz);
                    if data.ref_count <= 0 {
                        usage.volumes_reclaimable = usage.volumes_reclaimable.saturating_add(sz);
                    }
                }
            }
        }

        if let Some(cache_items) = self.build_cache {
            usage.build_cache_count = cache_items.len();
            for cache in cache_items {
                usage.build_cache_size = usage.build_cache_size.saturating_add(cache.size);
                if !cache.in_use {
                    usage.build_cache_reclaimable =
                        usage.build_cache_reclaimable.saturating_add(cache.size);
                }
            }
        }

        usage.recalculate_totals();
        usage
    }
}
