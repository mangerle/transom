//! 标签列表与 manifest 摘要查询。

use super::auth::{apply_auth, auth_for_scope};
use super::io::{read_error_text, read_json};
use super::{api_repository, encode_repository, http_client, registry_base, registry_error};
use crate::error::ContainerError;
use reqwest::header::ACCEPT;
use serde::Deserialize;
use transom_core::ContainerRegistry;

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
