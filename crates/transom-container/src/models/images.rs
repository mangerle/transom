//! 镜像列表、检视、历史、拉取/检索与清理的线格式结构体。

use super::{null_as_empty, parse_rfc3339_secs};
use serde::Deserialize;
use transom_core::{ContainerImageDetails, ContainerImageLayer, ContainerImageSummary};

/// `GET /images/json` 列表项。
#[derive(Debug, Clone, Deserialize)]
pub struct ImageListItem {
    #[serde(rename = "Id")]
    pub id: String,
    /// 悬空镜像为 null；用 `Option` 兜住。
    #[serde(rename = "RepoTags", default)]
    pub repo_tags: Option<Vec<String>>,
    #[serde(rename = "Created", default)]
    pub created: i64,
    #[serde(rename = "Size", default)]
    pub size: u64,
}

impl ImageListItem {
    pub fn into_summary(self) -> ContainerImageSummary {
        ContainerImageSummary {
            id: self.id,
            tags: self.repo_tags.unwrap_or_default(),
            size: self.size,
            created: self.created,
        }
    }
}

/// `GET /images/{id}/json` 中本模块关心的字段。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageInspect {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "RepoTags", default)]
    pub repo_tags: Option<Vec<String>>,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "Created", default)]
    pub created: Option<String>,
    #[serde(rename = "Os", default)]
    pub os: Option<String>,
    #[serde(rename = "Architecture", default)]
    pub architecture: Option<String>,
    #[serde(rename = "RootFS", default)]
    pub root_fs: Option<RootFs>,
    #[serde(rename = "RepoDigests", default)]
    pub repo_digests: Option<Vec<String>>,
    #[serde(rename = "ParentId", default)]
    pub parent_id: Option<String>,
    #[serde(rename = "VirtualSize", default)]
    pub virtual_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RootFs {
    #[serde(rename = "Layers", default)]
    pub layers: Vec<String>,
}

impl ImageInspect {
    /// 收敛为界面详情；`raw` 是引擎原始 JSON，供「原始 JSON」视图直接展示。
    pub fn into_details(self, raw: String) -> ContainerImageDetails {
        let layer_digests = self.root_fs.map(|root| root.layers).unwrap_or_default();
        ContainerImageDetails {
            id: self.id,
            tags: self.repo_tags.unwrap_or_default(),
            repo_digests: self.repo_digests.unwrap_or_default(),
            parent: self.parent_id.unwrap_or_default(),
            virtual_size: self.virtual_size.unwrap_or(0),
            inspect: raw,
            size: self.size,
            created: parse_rfc3339_secs(self.created.as_deref()).unwrap_or(0),
            os: self.os,
            architecture: self.architecture,
            layers: layer_digests.len(),
            layer_digests,
            history: Vec::new(),
        }
    }
}

/// `GET /images/{name}/history` 响应单项。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageHistoryItem {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "Created", default)]
    pub created: i64,
    #[serde(rename = "CreatedBy", default)]
    pub created_by: String,
    #[serde(rename = "Tags", default)]
    pub tags: Option<Vec<String>>,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "Comment", default)]
    pub comment: String,
}

impl ImageHistoryItem {
    pub fn into_layer(self) -> ContainerImageLayer {
        ContainerImageLayer {
            id: self.id,
            created: self.created,
            created_by: self.created_by,
            size: self.size,
            comment: self.comment,
            tags: self.tags.unwrap_or_default(),
        }
    }
}

/// `POST /images/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImagePruneResponse {
    /// 没有可删条目时引擎返回 `null`，不能交给 `default` 硬接。
    #[serde(rename = "ImagesDeleted", default, deserialize_with = "null_as_empty")]
    pub images_deleted: Vec<ImagePruneEntry>,
    #[serde(rename = "SpaceReclaimed", default)]
    pub space_reclaimed: u64,
}

/// 单条镜像清理记录：引擎按动作回报，取消标签与删除镜像是两种条目。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImagePruneEntry {
    #[serde(rename = "Untagged", default)]
    pub untagged: Option<String>,
    #[serde(rename = "Deleted", default)]
    pub deleted: Option<String>,
}

/// `POST /build/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BuildCachePruneResponse {
    #[serde(rename = "CachesDeleted", default, deserialize_with = "null_as_empty")]
    pub caches_deleted: Vec<String>,
    #[serde(rename = "SpaceReclaimed", default)]
    pub space_reclaimed: u64,
}

/// 镜像拉取事件；包含分层与整体进度。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImagePullEvent {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(rename = "errorDetail", default)]
    pub error_detail: Option<PullErrorDetail>,
    #[serde(rename = "progressDetail", default)]
    pub progress_detail: Option<ProgressDetail>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PullErrorDetail {
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProgressDetail {
    #[serde(default)]
    pub current: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
}

/// 远端镜像检索项（Docker Engine API GET /images/search 响应）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageSearchItem {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_official: bool,
    #[serde(default)]
    pub is_automated: bool,
    #[serde(default)]
    pub star_count: u64,
}

impl From<ImageSearchItem> for transom_core::ImageSearchResult {
    fn from(item: ImageSearchItem) -> Self {
        Self {
            name: item.name,
            description: item.description,
            is_official: item.is_official,
            is_automated: item.is_automated,
            star_count: item.star_count,
        }
    }
}
