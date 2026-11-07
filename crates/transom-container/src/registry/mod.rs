//! 镜像仓库（Registry v2 / Docker Hub）检索与标签查询。
//!
//! 设计原理（Why）：
//! - daemon 的 `/images/search` 只覆盖 Docker Hub，私有仓库必须由客户端直接走
//!   Registry HTTP API；
//! - Registry v2 鉴权是标准 Bearer Challenge：先探测 `/v2/` 取 realm/service，
//!   再用 Basic 凭据换取短期 token；
//! - HTTP 客户端为进程级单例，复用连接池与 TLS 会话；
//! - 响应体读取有硬上限。

mod auth;
mod io;
mod search;
mod tags;

pub use search::search;
pub use tags::{list_tags, manifest_digest};

use crate::error::ContainerError;
use crate::http::percent_encode;
use reqwest::Client;
use std::sync::OnceLock;
use std::time::Duration;
use transom_core::ContainerRegistry;

/// 单次 Registry 请求总超时。
const REGISTRY_TIMEOUT: Duration = Duration::from_secs(30);

/// JSON 响应上限：标签列表与搜索结果的合理上限。
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;

/// 错误响应体保留上限。
const MAX_ERROR_BYTES: usize = 4 * 1024;

/// Docker Hub 的 Registry v2 入口。
const DOCKER_HUB_REGISTRY: &str = "https://registry-1.docker.io";

/// Docker Hub 的检索 API。
const DOCKER_HUB_API: &str = "https://hub.docker.com/v2";

/// 进程级 HTTP 客户端：reqwest 自带连接池，禁止每次请求重新创建。
pub(super) fn http_client() -> Result<&'static Client, ContainerError> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent("Transom")
            .timeout(REGISTRY_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())
    });
    match client {
        Ok(client) => Ok(client),
        Err(message) => Err(registry_error("初始化镜像仓库客户端", message.clone())),
    }
}

pub(super) fn registry_error(
    operation: impl Into<String>,
    message: impl Into<String>,
) -> ContainerError {
    ContainerError::Registry {
        operation: operation.into(),
        message: message.into(),
    }
}

/// 仓库 API 基地址；去掉末尾斜杠与误填的 `/v2` 后缀。
pub(super) fn registry_base(registry: &ContainerRegistry) -> Result<String, ContainerError> {
    if registry.is_docker_hub() {
        return Ok(DOCKER_HUB_REGISTRY.to_string());
    }
    let raw = registry.url.trim();
    if raw.is_empty() {
        return Err(registry_error("解析镜像仓库地址", "仓库地址为空"));
    }
    let with_scheme = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let trimmed = with_scheme.trim_end_matches('/');
    let base = trimmed.strip_suffix("/v2").unwrap_or(trimmed);
    Ok(base.trim_end_matches('/').to_string())
}

/// Registry API 使用的仓库名；Docker Hub 官方镜像需要补 `library/` 前缀。
pub(super) fn api_repository(registry: &ContainerRegistry, repository: &str) -> String {
    let repo = repository.trim().trim_start_matches('/');
    if registry.is_docker_hub() && !repo.contains('/') {
        format!("library/{repo}")
    } else {
        repo.to_string()
    }
}

pub(super) fn encode_repository(repository: &str) -> String {
    repository
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_encode)
        .collect::<Vec<_>>()
        .join("/")
}
