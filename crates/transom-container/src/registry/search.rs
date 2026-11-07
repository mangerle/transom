//! Docker Hub / Harbor / Registry 目录检索。

use super::auth::{apply_auth, auth_for_scope};
use super::io::read_json;
use super::{DOCKER_HUB_API, http_client, registry_base, registry_error};
use crate::error::ContainerError;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;
use transom_core::{ContainerRegistry, ImageSearchResult};

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

#[derive(Debug, Deserialize, Default)]
struct CatalogResponse {
    #[serde(default)]
    repositories: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::super::api_repository;
    use super::*;

    #[test]
    fn docker_hub_repository_gets_library_prefix_for_registry_api() {
        let hub = ContainerRegistry::docker_hub();
        assert_eq!(api_repository(&hub, "nginx"), "library/nginx");
        assert_eq!(api_repository(&hub, "bitnami/redis"), "bitnami/redis");
    }
}
