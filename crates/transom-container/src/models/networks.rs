//! 网络列表、检视、连接信息与清理的线格式结构体。

use super::{null_as_empty, sorted_pairs};
use serde::Deserialize;
use std::collections::HashMap;
use transom_core::{
    ContainerNetworkAttachment, ContainerNetworkDetails, ContainerNetworkSummary, short_id,
};

/// `GET /networks` 列表项。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkItem {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Driver", default)]
    pub driver: String,
    #[serde(rename = "Scope", default)]
    pub scope: String,
    #[serde(rename = "IPAM", default)]
    pub ipam: Option<Ipam>,
    #[serde(rename = "Containers", default)]
    pub containers: Option<HashMap<String, NetworkContainerItem>>,
    #[serde(rename = "Internal", default)]
    pub internal: bool,
    #[serde(rename = "Attachable", default)]
    pub attachable: bool,
    #[serde(rename = "EnableIPv6", default)]
    pub enable_ipv6: bool,
    /// 引擎在无标签时返回 `null`，`HashMap` 的 `default` 接不住显式 null。
    #[serde(rename = "Labels", default, deserialize_with = "null_as_empty")]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Ipam {
    /// 内置网络（bridge/host/none）会返回 `"Config": null`；`Vec` 的 `default`
    /// 只覆盖字段缺失，接不住显式 null，曾经导致网络列表整体解析失败。
    #[serde(rename = "Config", default, deserialize_with = "null_as_empty")]
    pub config: Vec<IpamConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct IpamConfig {
    #[serde(rename = "Subnet", default)]
    pub subnet: Option<String>,
    #[serde(rename = "Gateway", default)]
    pub gateway: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkContainerItem {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "IPv4Address", default)]
    pub ipv4_address: Option<String>,
}

impl NetworkItem {
    pub fn summary(&self) -> ContainerNetworkSummary {
        ContainerNetworkSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            driver: self.driver.clone(),
            scope: self.scope.clone(),
            subnet: self
                .ipam
                .as_ref()
                .and_then(|ipam| ipam.config.first())
                .and_then(|config| config.subnet.clone()),
        }
    }

    /// 收敛为界面详情；`raw` 是引擎原始 JSON，供「原始 JSON」视图直接展示。
    pub fn into_details(self, raw: String) -> ContainerNetworkDetails {
        let summary = self.summary();
        let gateway = self
            .ipam
            .as_ref()
            .and_then(|ipam| ipam.config.first())
            .and_then(|config| config.gateway.clone());
        let mut connected: Vec<ContainerNetworkAttachment> = self
            .containers
            .unwrap_or_default()
            .into_iter()
            .map(|(id, item)| ContainerNetworkAttachment {
                id: short_id(&id),
                name: item.name,
                ipv4: item
                    .ipv4_address
                    .map(|address| address.trim_end_matches('/').to_string())
                    .filter(|address| !address.is_empty()),
            })
            .collect();
        connected.sort_by(|left, right| left.name.cmp(&right.name));
        ContainerNetworkDetails {
            summary,
            gateway,
            connected,
            internal: self.internal,
            attachable: self.attachable,
            enable_ipv6: self.enable_ipv6,
            labels: sorted_pairs(self.labels),
            inspect: raw,
        }
    }
}

/// `POST /networks/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkPruneResponse {
    /// 清理掉的网络名列表；没有可删条目时是 `null`。
    #[serde(
        rename = "NetworksDeleted",
        default,
        deserialize_with = "null_as_empty"
    )]
    pub networks_deleted: Vec<String>,
}
