//! Pod 摘要与 Pod 级动作（Podman 专有）。

use serde::{Deserialize, Serialize};

/// Podman Pod 摘要；Docker 没有对应概念。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerPodSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created: String,
    /// Pod 内容器数量。
    #[serde(default)]
    pub containers: u32,
}

/// Podman Pod 操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerPodAction {
    Start,
    Stop,
    Restart,
    Remove,
}
