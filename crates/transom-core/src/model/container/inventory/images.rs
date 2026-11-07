//! 镜像摘要、详情、层信息与仓库凭据。

use crate::model::container::short_id;
use serde::{Deserialize, Serialize};

/// 镜像列表项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerImageSummary {
    pub id: String,
    /// `Repo:Tag` 列表；悬空镜像可能为空。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 字节数。
    pub size: u64,
    pub created: i64,
}

impl ContainerImageSummary {
    /// 界面主标题：优先仓库标签，悬空镜像回落到短 ID。
    pub fn display_name(&self) -> String {
        self.tags
            .first()
            .cloned()
            .unwrap_or_else(|| short_id(&self.id))
    }

    /// 列表用的短标题：剥掉仓库主机与官方 `library/` 前缀，只留 `名字[:标签]`。
    ///
    /// 设计原理（Why）：走国内加速仓库时完整引用动辄三四十字符
    /// （`docker.m.daocloud.io/library/busybox:1.36`），在任何宽度的列表里都会被
    /// 截断成不可读的片段；Docker Desktop 与 Portainer 都是「短名做主标题、完整
    /// 引用留在详情」。代价是同一行看不到来源仓库，因此详情基础信息与起容器时
    /// 填的镜像参数始终使用 [`Self::display_name`] 的完整引用。
    pub fn short_display_name(&self) -> String {
        self.tags
            .first()
            .map(|tag| shorten_reference(tag))
            .unwrap_or_else(|| short_id(&self.id))
    }
}

/// 把镜像引用缩短为可读主干：去掉仓库主机段与官方 `library/` 前缀。
///
/// 判定沿用 Docker 自己的规则：只有第一段含 `.`、`:` 或等于 `localhost` 时才是
/// 仓库主机，否则它是命名空间（如 `ghcr.io/foo/bar` 的 `foo`）必须保留。
fn shorten_reference(reference: &str) -> String {
    let rest = match reference.split_once('/') {
        Some((head, tail)) if head.contains('.') || head.contains(':') || head == "localhost" => {
            tail
        }
        _ => reference,
    };
    rest.strip_prefix("library/").unwrap_or(rest).to_string()
}

/// 镜像详情（基础信息页签）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerImageDetails {
    pub id: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 仓库摘要（`名字@sha256:…`）：同一镜像被多个 tag 指向时，只有这里能看清来源。
    #[serde(default)]
    pub repo_digests: Vec<String>,
    /// 父镜像 ID；OCI 镜像通常为空串。
    #[serde(default)]
    pub parent: String,
    /// 解包后的虚拟大小（字节）；引擎不回报时为 0。
    #[serde(default)]
    pub virtual_size: u64,
    pub size: u64,
    pub created: i64,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub architecture: Option<String>,
    /// 层数量；引擎响应缺失时为 0。
    #[serde(default)]
    pub layers: usize,
    /// 每一层的 digest（自底向上）：详情页逐层展示，只有一个数字太抽象。
    #[serde(default)]
    pub layer_digests: Vec<String>,
    /// 原始 inspect JSON：界面「原始 JSON」视图用，引擎未返回时为空串。
    #[serde(default)]
    pub inspect: String,
    /// 镜像构建历史层。
    #[serde(default)]
    pub history: Vec<ContainerImageLayer>,
}

/// 镜像单层构建历史记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerImageLayer {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 容器 Registry 认证凭据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerRegistryAuth {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub server_address: Option<String>,
    #[serde(default)]
    pub identity_token: Option<String>,
}
