//! 数据卷摘要与详情。

use serde::{Deserialize, Serialize};

/// 数据卷摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerVolumeSummary {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    /// 驱动选项（`local` 的 `type`/`device` 等）：排查卷为什么挂不上时要看。
    #[serde(default)]
    pub options: Vec<(String, String)>,
}

/// 数据卷详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerVolumeDetails {
    pub summary: ContainerVolumeSummary,
    /// 正在使用该卷的容器名。
    #[serde(default)]
    pub used_by: Vec<String>,
    /// 原始 inspect JSON：界面「原始 JSON」视图用。
    #[serde(default)]
    pub inspect: String,
}
