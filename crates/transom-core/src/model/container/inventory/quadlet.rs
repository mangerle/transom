//! Quadlet 单元与单元级动作。

use serde::{Deserialize, Serialize};

/// Podman Quadlet 单元；由 systemd 用户单元运行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerQuadletUnit {
    /// systemd 单元名，如 `web.service`。
    pub name: String,
    /// Quadlet 源文件路径。
    pub path: String,
    /// 文件类型：container/pod/volume/network/kube。
    pub kind: String,
    /// `systemctl is-active` 结果；无法查询时为 `None`。
    #[serde(default)]
    pub active: Option<bool>,
}

/// Quadlet 单元操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerQuadletAction {
    Start,
    Stop,
    Restart,
}
