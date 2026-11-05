//! 容器镜像仓库（Registry）认证解析与凭证管理。
//!
//! 设计原理（Why）：用户在向私有 Registry 或 Docker Hub 推送镜像时，
//! Docker REST API 要求在 `X-Registry-Auth` 请求头附带 base64 编码的凭据 JSON。
//! 优先读取明确传入的凭据，未传入时自动从宿主机 `~/.docker/config.json` 解析已登录的
//! 凭证（包含 Docker Hub 与私有 Registry），实现免密无缝推送。

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use transom_core::ContainerRegistryAuth;

#[derive(Debug, Clone, Deserialize, Default)]
struct DockerConfigFile {
    #[serde(default)]
    auths: HashMap<String, DockerConfigAuthEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DockerConfigAuthEntry {
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    identitytoken: Option<String>,
}

#[derive(Serialize)]
struct DockerAuthPayload<'a> {
    username: &'a str,
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    serveraddress: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identitytoken: Option<&'a str>,
}

/// 将凭据编码为 `X-Registry-Auth` 所需的 base64 字符串。
pub fn encode_auth_header(auth: &ContainerRegistryAuth) -> Result<String, String> {
    let payload = DockerAuthPayload {
        username: &auth.username,
        password: &auth.password,
        serveraddress: auth.server_address.as_deref(),
        identitytoken: auth.identity_token.as_deref(),
    };
    let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    Ok(BASE64.encode(json))
}

/// 解析目标镜像的仓库服务器地址。
///
/// 例如：
/// - `registry.example.com/app/web:v1` -> `registry.example.com`
/// - `localhost:5000/my-image` -> `localhost:5000`
/// - `ubuntu:latest` 或 `nginx` -> `https://index.docker.io/v1/`
pub fn extract_registry_host(image: &str) -> String {
    let slash = image.find('/');
    let first_part = match slash {
        Some(idx) => &image[..idx],
        None => return "https://index.docker.io/v1/".to_string(),
    };
    if first_part.contains('.') || first_part.contains(':') || first_part == "localhost" {
        first_part.to_string()
    } else {
        "https://index.docker.io/v1/".to_string()
    }
}

/// 尝试解析对应镜像仓库的认证凭据，并返回 base64 格式的 Header 字符串。
pub fn resolve_auth_header(
    image: &str,
    explicit_auth: Option<&ContainerRegistryAuth>,
) -> Option<String> {
    if let Some(auth) = explicit_auth {
        return encode_auth_header(auth).ok();
    }
    // 从 ~/.docker/config.json 自动匹配
    let config_path = home_dir()?.join(".docker").join("config.json");
    let content = std::fs::read_to_string(config_path).ok()?;
    let config: DockerConfigFile = serde_json::from_str(&content).ok()?;
    let host = extract_registry_host(image);

    // 尝试直接匹配或去除协议头匹配
    let entry = config.auths.get(&host).or_else(|| {
        let stripped = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        config.auths.iter().find_map(|(k, v)| {
            let k_stripped = k
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            if k_stripped == stripped {
                Some(v)
            } else {
                None
            }
        })
    })?;

    if let Some(token) = &entry.identitytoken {
        let payload = DockerAuthPayload {
            username: "",
            password: "",
            serveraddress: Some(&host),
            identitytoken: Some(token),
        };
        if let Ok(json) = serde_json::to_string(&payload) {
            return Some(BASE64.encode(json));
        }
    }

    if let Some(auth_b64) = &entry.auth {
        if let Ok(decoded) = BASE64.decode(auth_b64) {
            if let Ok(s) = String::from_utf8(decoded) {
                if let Some((user, pass)) = s.split_once(':') {
                    let payload = DockerAuthPayload {
                        username: user,
                        password: pass,
                        serveraddress: Some(&host),
                        identitytoken: None,
                    };
                    if let Ok(json) = serde_json::to_string(&payload) {
                        return Some(BASE64.encode(json));
                    }
                }
            }
        }
    }

    None
}

fn home_dir() -> Option<PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_registry_host() {
        assert_eq!(
            extract_registry_host("registry.example.com/org/app:v1"),
            "registry.example.com"
        );
        assert_eq!(
            extract_registry_host("localhost:5000/my-app:latest"),
            "localhost:5000"
        );
        assert_eq!(
            extract_registry_host("ubuntu:22.04"),
            "https://index.docker.io/v1/"
        );
        assert_eq!(
            extract_registry_host("library/nginx"),
            "https://index.docker.io/v1/"
        );
    }

    #[test]
    fn test_encode_auth_header() {
        let auth = ContainerRegistryAuth {
            username: "testuser".to_string(),
            password: "testpassword".to_string(),
            server_address: Some("https://index.docker.io/v1/".to_string()),
            identity_token: None,
        };
        let header = encode_auth_header(&auth).expect("编码成功");
        let decoded = BASE64.decode(header).expect("解码成功");
        let json: serde_json::Value = serde_json::from_slice(&decoded).expect("解析成功");
        assert_eq!(json["username"], "testuser");
        assert_eq!(json["password"], "testpassword");
        assert_eq!(json["serveraddress"], "https://index.docker.io/v1/");
    }
}
