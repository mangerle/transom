//! 数据卷列表、检视与清理的线格式结构体。

use super::{null_as_empty, sorted_pairs};
use serde::Deserialize;
use std::collections::HashMap;
use transom_core::ContainerVolumeSummary;

/// `GET /volumes` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VolumeListResponse {
    #[serde(rename = "Volumes", default)]
    pub volumes: Vec<VolumeItem>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VolumeItem {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Driver", default)]
    pub driver: String,
    #[serde(rename = "Mountpoint", default)]
    pub mountpoint: String,
    #[serde(rename = "CreatedAt", default)]
    pub created_at: Option<String>,
    #[serde(rename = "Labels", default, deserialize_with = "null_as_empty")]
    pub labels: HashMap<String, String>,
    #[serde(rename = "Options", default, deserialize_with = "null_as_empty")]
    pub options: HashMap<String, String>,
}

impl VolumeItem {
    pub fn into_summary(self) -> ContainerVolumeSummary {
        ContainerVolumeSummary {
            name: self.name,
            driver: self.driver,
            mountpoint: self.mountpoint,
            created: self.created_at,
            labels: sorted_pairs(self.labels),
            options: sorted_pairs(self.options),
        }
    }
}

/// `POST /volumes/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VolumePruneResponse {
    /// 同镜像清理：没有可删条目时是 `null`。
    #[serde(rename = "VolumesDeleted", default, deserialize_with = "null_as_empty")]
    pub volumes_deleted: Vec<String>,
    #[serde(rename = "SpaceReclaimed", default)]
    pub space_reclaimed: u64,
}
