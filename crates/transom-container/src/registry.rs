//! 镜像仓库（Registry v2 / Docker Hub）检索与标签查询。
//!
//! 设计原理（Why）：
//! - Docker daemon 的 `/images/search` 只检索 Docker Hub，无法替用户搜索 Harbor、
//!   GHCR 等私有仓库；因此搜索与标签列表必须由客户端直接走 Registry HTTP API；
//! - Registry v2 的鉴权是标准 Bearer Challenge 流程：先探测 `/v2/`，从
//!   `WWW-Authenticate` 取 realm/service，再用 Basic 凭据换取短期 token；
//! - HTTP 客户端做成进程级单例，复用连接池与 TLS 会话，避免每次搜索重新握手；
//! - 响应体读取有硬上限，JSON 解析前不会先把任意大小的响应读入内存。

use crate::error::ContainerError;
use crate::http::percent_encode;
use futures::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Duration;
use transom_core::{ContainerRegistry, ImageSearchResult};

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
fn http_client() -> Result<&'static Client, ContainerError> {
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

fn registry_error(operation: impl Into<String>, message: impl Into<String>) -> ContainerError {
    ContainerError::Registry {
        operation: operation.into(),
        message: message.into(),
    }
}

/// 仓库 API 基地址；去掉末尾斜杠与误填的 `/v2` 后缀。
fn registry_base(registry: &ContainerRegistry) -> Result<String, ContainerError> {
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
fn api_repository(registry: &ContainerRegistry, repository: &str) -> String {
    let repo = repository.trim().trim_start_matches('/');
    if registry.is_docker_hub() && !repo.contains('/') {
        format!("library/{repo}")
    } else {
        repo.to_string()
    }
}

/// 按仓库类型检索镜像。
///
/// Docker Hub 走官方 Hub Search API；私有仓库优先尝试 Harbor 搜索 API，
/// 再回落到 Registry v2 的 `/v2/_catalog`，并做客户端关键字过滤。
pub async fn search(
    registry: &ContainerRegistry,
    term: &str,
    limit: Option<u32>,
) -> Result<Vec<ImageSearchResult>, ContainerError> {
    let term = term.trim();
    if term.is_empty() {
        return Ok(Vec::new());
    }
    let client = http_client()?;
    if registry.is_docker_hub() {
        return search_docker_hub(client, term, limit).await;
    }
    let harbor = search_harbor(client, registry, term, limit).await;
    match harbor {
        Ok(results) if !results.is_empty() => Ok(results),
        Ok(_) => search_catalog(client, registry, term, limit).await,
        Err(harbor_error) => {
            log::debug!("Harbor 搜索不可用，回落到 Registry 目录检索，原因: {harbor_error}");
            match search_catalog(client, registry, term, limit).await {
                Ok(results) => Ok(results),
                Err(catalog_error) => Err(registry_error(
                    "搜索镜像",
                    format!(
                        "{} 不支持镜像搜索或鉴权失败（Harbor: {}；Catalog: {}）",
                        registry.name, harbor_error, catalog_error
                    ),
                )),
            }
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct HubSearchResponse {
    #[serde(default)]
    results: Vec<HubSearchItem>,
}

#[derive(Debug, Deserialize, Default)]
struct HubSearchItem {
    #[serde(default, rename = "repo_name")]
    name: String,
    #[serde(default, rename = "short_description")]
    description: String,
    #[serde(default, rename = "star_count")]
    star_count: u64,
    #[serde(default, rename = "is_official")]
    is_official: bool,
    #[serde(default, rename = "is_automated")]
    is_automated: bool,
}

async fn search_docker_hub(
    client: &Client,
    term: &str,
    limit: Option<u32>,
) -> Result<Vec<ImageSearchResult>, ContainerError> {
    let page_size = limit.unwrap_or(25).clamp(1, 100).to_string();
    let response = client
        .get(format!("{DOCKER_HUB_API}/search/repositories/"))
        .query(&[("query", term), ("page_size", page_size.as_str())])
        .send()
        .await
        .map_err(|error| registry_error("搜索 Docker Hub 镜像", error.to_string()))?;
    let parsed: HubSearchResponse = read_json(response, "搜索 Docker Hub 镜像").await?;
    let mut results = Vec::with_capacity(parsed.results.len());
    let mut seen = HashSet::with_capacity(parsed.results.len());
    for item in parsed.results {
        let name = item.name.trim().trim_start_matches('/').to_string();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        results.push(ImageSearchResult {
            name,
            description: item.description,
            is_official: item.is_official,
            is_automated: item.is_automated,
            star_count: item.star_count,
        });
    }
    Ok(results)
}

#[derive(Debug, Deserialize, Default)]
struct HarborSearchResponse {
    #[serde(default)]
    repository: Vec<HarborRepository>,
}

#[derive(Debug, Deserialize, Default)]
struct HarborRepository {
    #[serde(default)]
    repository_name: String,
    #[serde(default)]
    project_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    pull_count: u64,
}

async fn search_harbor(
    client: &Client,
    registry: &ContainerRegistry,
    term: &str,
    limit: Option<u32>,
) -> Result<Vec<ImageSearchResult>, ContainerError> {
    let base = registry_base(registry)?;
    let mut request = client
        .get(format!("{base}/api/v2.0/search"))
        .query(&[("q", term)]);
    if !registry.username.is_empty() {
        request = request.basic_auth(&registry.username, Some(&registry.password));
    }
    let response = request
        .send()
        .await
        .map_err(|error| registry_error("搜索 Harbor 镜像", error.to_string()))?;
    let parsed: HarborSearchResponse = read_json(response, "搜索 Harbor 镜像").await?;
    let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
    let mut results = Vec::with_capacity(parsed.repository.len().min(limit));
    let mut seen = HashSet::with_capacity(parsed.repository.len());
    for item in parsed.repository {
        let name = item
            .repository_name
            .trim()
            .trim_start_matches('/')
            .to_string();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        results.push(ImageSearchResult {
            name,
            description: item.description,
            is_official: item.project_name.eq_ignore_ascii_case("library"),
            is_automated: false,
            star_count: item.pull_count,
        });
        if results.len() >= limit {
            break;
        }
    }
    Ok(results)
}

#[derive(Debug, Deserialize, Default)]
struct CatalogResponse {
    #[serde(default)]
    repositories: Vec<String>,
}

async fn search_catalog(
    client: &Client,
    registry: &ContainerRegistry,
    term: &str,
    limit: Option<u32>,
) -> Result<Vec<ImageSearchResult>, ContainerError> {
    let base = registry_base(registry)?;
    let auth = auth_for_scope(client, registry, "registry:catalog:*").await?;
    let request = apply_auth(
        client.get(format!("{base}/v2/_catalog?n=200")),
        &auth,
        registry,
    );
    let response = request
        .send()
        .await
        .map_err(|error| registry_error("列出镜像仓库目录", error.to_string()))?;
    let parsed: CatalogResponse = read_json(response, "列出镜像仓库目录").await?;
    let needle = term.to_lowercase();
    let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
    let mut results = Vec::with_capacity(parsed.repositories.len().min(limit));
    let mut seen = HashSet::with_capacity(parsed.repositories.len());
    for name in parsed.repositories {
        let name = name.trim().trim_start_matches('/').to_string();
        if name.is_empty() || !name.to_lowercase().contains(&needle) || !seen.insert(name.clone()) {
            continue;
        }
        results.push(ImageSearchResult {
            name,
            ..ImageSearchResult::default()
        });
        if results.len() >= limit {
            break;
        }
    }
    Ok(results)
}

#[derive(Debug, Deserialize, Default)]
struct TagListResponse {
    #[serde(default)]
    tags: Option<Vec<String>>,
}

/// 读取仓库的实时标签列表；同一仓库的标签只来自 Registry v2。
pub async fn list_tags(
    registry: &ContainerRegistry,
    repository: &str,
) -> Result<Vec<String>, ContainerError> {
    let client = http_client()?;
    let base = registry_base(registry)?;
    let repo = api_repository(registry, repository);
    if repo.is_empty() {
        return Err(registry_error("读取镜像标签", "仓库名为空"));
    }
    let scope = format!("repository:{repo}:pull");
    let auth = auth_for_scope(client, registry, &scope).await?;
    let encoded = encode_repository(&repo);
    let request = apply_auth(
        client.get(format!("{base}/v2/{encoded}/tags/list?n=1000")),
        &auth,
        registry,
    );
    let response = request
        .send()
        .await
        .map_err(|error| registry_error("读取镜像标签", error.to_string()))?;
    let parsed: TagListResponse = read_json(response, "读取镜像标签").await?;
    let mut tags: Vec<String> = parsed
        .tags
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    // `latest` 是用户最常拉取的档位，固定放在首位。
    if let Some(index) = tags.iter().position(|tag| tag == "latest") {
        let latest = tags.remove(index);
        tags.insert(0, latest);
    }
    Ok(tags)
}

fn encode_repository(repository: &str) -> String {
    repository
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_encode)
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, Clone)]
enum RegistryAuth {
    None,
    Bearer(String),
    Basic,
}

fn apply_auth(
    request: reqwest::RequestBuilder,
    auth: &RegistryAuth,
    registry: &ContainerRegistry,
) -> reqwest::RequestBuilder {
    match auth {
        RegistryAuth::None => request,
        RegistryAuth::Bearer(token) => request.header(AUTHORIZATION, format!("Bearer {token}")),
        RegistryAuth::Basic => request.basic_auth(&registry.username, Some(&registry.password)),
    }
}

#[derive(Debug, Deserialize, Default)]
struct TokenResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

async fn auth_for_scope(
    client: &Client,
    registry: &ContainerRegistry,
    scope: &str,
) -> Result<RegistryAuth, ContainerError> {
    let base = registry_base(registry)?;
    let response = client
        .get(format!("{base}/v2/"))
        .send()
        .await
        .map_err(|error| registry_error("探测镜像仓库鉴权", error.to_string()))?;
    let status = response.status();
    if status.is_success() {
        return Ok(if registry.username.is_empty() {
            RegistryAuth::None
        } else {
            RegistryAuth::Basic
        });
    }
    if status != StatusCode::UNAUTHORIZED {
        let body = read_error_text(response).await;
        return Err(registry_error(
            "探测镜像仓库鉴权",
            format!("HTTP {status}: {body}"),
        ));
    }

    let challenge = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let Some(challenge) = challenge else {
        return basic_or_error(registry);
    };
    if !challenge.to_ascii_lowercase().starts_with("bearer") {
        return basic_or_error(registry);
    }
    let params = parse_challenge_params(&challenge)
        .ok_or_else(|| registry_error("解析镜像仓库鉴权", "Bearer 挑战缺少参数"))?;
    let realm = params
        .get("realm")
        .cloned()
        .ok_or_else(|| registry_error("解析镜像仓库鉴权", "Bearer 挑战缺少 realm"))?;
    let mut query: Vec<(&str, &str)> = Vec::with_capacity(2);
    if let Some(service) = params.get("service") {
        query.push(("service", service.as_str()));
    }
    if !scope.is_empty() {
        query.push(("scope", scope));
    }
    let mut request = client.get(realm).query(&query);
    if !registry.username.is_empty() {
        request = request.basic_auth(&registry.username, Some(&registry.password));
    }
    let response = request
        .send()
        .await
        .map_err(|error| registry_error("获取镜像仓库访问令牌", error.to_string()))?;
    let token: TokenResponse = read_json(response, "获取镜像仓库访问令牌").await?;
    let token = token
        .token
        .or(token.access_token)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| registry_error("获取镜像仓库访问令牌", "令牌响应为空"))?;
    Ok(RegistryAuth::Bearer(token))
}

fn basic_or_error(registry: &ContainerRegistry) -> Result<RegistryAuth, ContainerError> {
    if registry.username.is_empty() {
        Err(registry_error(
            "镜像仓库鉴权",
            "仓库要求认证，但未配置用户名与密码",
        ))
    } else {
        Ok(RegistryAuth::Basic)
    }
}

/// 解析 `Bearer realm="...",service="...",scope="..."` 形式的挑战头。
fn parse_challenge_params(header: &str) -> Option<HashMap<String, String>> {
    let rest = header
        .trim()
        .strip_prefix("Bearer")
        .or_else(|| header.trim().strip_prefix("bearer"))?
        .trim();
    let mut params = HashMap::with_capacity(3);
    for part in rest.split(',') {
        let (key, value) = part.split_once('=')?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches('"').to_string();
        if !key.is_empty() {
            params.insert(key, value);
        }
    }
    Some(params)
}

async fn read_json<T: DeserializeOwned>(
    response: Response,
    operation: &str,
) -> Result<T, ContainerError> {
    let status = response.status();
    if !status.is_success() {
        let body = read_error_text(response).await;
        return Err(registry_error(operation, format!("HTTP {status}: {body}")));
    }
    let bytes = read_bytes_limited(response, MAX_JSON_BYTES, operation).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| registry_error(operation, format!("响应解析失败: {error}")))
}

async fn read_bytes_limited(
    response: Response,
    limit: usize,
    operation: &str,
) -> Result<Vec<u8>, ContainerError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(8192.min(limit));
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| registry_error(operation, error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(registry_error(
                operation,
                format!("响应超过上限 {limit} 字节"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn read_error_text(response: Response) -> String {
    match read_bytes_limited(response, MAX_ERROR_BYTES, "读取仓库错误响应").await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(error) => error.to_string(),
    }
}

/// 读取指定 tag 的 manifest 摘要；用于将来展示 digest 与多架构信息。
pub async fn manifest_digest(
    registry: &ContainerRegistry,
    repository: &str,
    reference: &str,
) -> Result<Option<String>, ContainerError> {
    let client = http_client()?;
    let base = registry_base(registry)?;
    let repo = api_repository(registry, repository);
    if repo.is_empty() || reference.trim().is_empty() {
        return Err(registry_error("读取镜像摘要", "仓库名或标签为空"));
    }
    let scope = format!("repository:{repo}:pull");
    let auth = auth_for_scope(client, registry, &scope).await?;
    let encoded = encode_repository(&repo);
    let request = apply_auth(
        client
            .head(format!("{base}/v2/{encoded}/manifests/{reference}"))
            .header(
                ACCEPT,
                "application/vnd.docker.distribution.manifest.v2+json, \
                 application/vnd.docker.distribution.manifest.list.v2+json, \
                 application/vnd.oci.image.manifest.v1+json, \
                 application/vnd.oci.image.index.v1+json",
            ),
        &auth,
        registry,
    );
    let response = request
        .send()
        .await
        .map_err(|error| registry_error("读取镜像摘要", error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = read_error_text(response).await;
        return Err(registry_error(
            "读取镜像摘要",
            format!("HTTP {status}: {body}"),
        ));
    }
    Ok(response
        .headers()
        .get("docker-content-digest")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::{api_repository, parse_challenge_params};
    use transom_core::ContainerRegistry;

    #[test]
    fn docker_hub_repository_gets_library_prefix_for_registry_api() {
        let hub = ContainerRegistry::docker_hub();
        assert_eq!(api_repository(&hub, "nginx"), "library/nginx");
        assert_eq!(api_repository(&hub, "bitnami/redis"), "bitnami/redis");
    }

    #[test]
    fn bearer_challenge_parses_quoted_parameters() {
        let params = parse_challenge_params(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull""#,
        )
        .expect("挑战头必须可解析");
        assert_eq!(
            params.get("realm").map(String::as_str),
            Some("https://auth.docker.io/token")
        );
        assert_eq!(
            params.get("service").map(String::as_str),
            Some("registry.docker.io")
        );
    }
}
