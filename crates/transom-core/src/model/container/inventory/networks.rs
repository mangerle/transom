//! 网络摘要、详情与容器接入信息。

use serde::{Deserialize, Serialize};

/// 网络摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerNetworkSummary {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    #[serde(default)]
    pub subnet: Option<String>,
}

/// 网络上的容器连接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerNetworkAttachment {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub ipv4: Option<String>,
}

/// 网络详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerNetworkDetails {
    pub summary: ContainerNetworkSummary,
    #[serde(default)]
    pub gateway: Option<String>,
    #[serde(default)]
    pub connected: Vec<ContainerNetworkAttachment>,
    /// 仅宿主机内部可见（`--internal`）。
    #[serde(default)]
    pub internal: bool,
    /// 允许运行中的容器动态接入（`--attachable`）。
    #[serde(default)]
    pub attachable: bool,
    /// 启用了 IPv6 段。
    #[serde(default)]
    pub enable_ipv6: bool,
    /// 网络标签：排查编排自动创建的网络时要看归属。
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    /// 原始 inspect JSON：界面「原始 JSON」视图用。
    #[serde(default)]
    pub inspect: String,
}
