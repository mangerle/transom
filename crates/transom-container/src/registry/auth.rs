//! Registry v2 Bearer/Basic 鉴权。

use super::io::{read_error_text, read_json};
use super::{registry_base, registry_error};
use crate::error::ContainerError;
use reqwest::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::collections::HashMap;
use transom_core::ContainerRegistry;

#[derive(Debug, Clone)]
pub(super) enum RegistryAuth {
    None,
    Bearer(String),
    Basic,
}

pub(super) fn apply_auth(
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

pub(super) async fn auth_for_scope(
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

#[derive(Debug, Deserialize, Default)]
struct TokenResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
