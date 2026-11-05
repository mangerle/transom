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
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use transom_core::ContainerRegistryAuth;

/// 外部凭据助手可能等待系统钥匙串授权，必须有上限，不能阻塞拉取。
const CREDENTIAL_HELPER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize, Default)]
struct DockerConfigFile {
    #[serde(default)]
    auths: HashMap<String, DockerConfigAuthEntry>,
    #[serde(default, rename = "credsStore")]
    creds_store: Option<String>,
    #[serde(default, rename = "credHelpers")]
    cred_helpers: HashMap<String, String>,
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

/// 通过 Docker 凭据助手（Credential Helper）查询凭据
///
/// ### 设计初衷 (Why)
/// 现代 Docker Desktop 以及主流操作系统默认将 Registry 凭据托管在系统级安全凭据库中
/// （如 osxkeychain, wincred, secretservice, pass 或 desktop）。
/// 此时 `~/.docker/config.json` 的 `auths` 中仅有空对象，必须通过标准命令
/// `docker-credential-<helper> get` 从外部助手获取明文与 Token。
///
/// ### 核心优势
/// 遵循 Docker 官方标准外部凭据交互契约，自动解析系统密钥链，避免私有仓库拉取鉴权失败。
///
/// ### 代价与局限
/// 依赖宿主机环境安装并配置有相应的 `docker-credential-*` 可执行程序；未安装时安全降级。
async fn query_credential_helper(helper: &str, server_url: &str) -> Option<(String, String)> {
    let program = format!("docker-credential-{helper}");
    let mut child = tokio::process::Command::new(&program)
        .arg("get")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            log::debug!("未能启动 Docker 凭证助手 {program}，原因: {error}");
            error
        })
        .ok()?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(server_url.as_bytes()).await;
        let _ = stdin.write_all(b"\n").await;
        let _ = stdin.shutdown().await;
    }

    let output = tokio::time::timeout(CREDENTIAL_HELPER_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        log::debug!(
            "Docker 凭证助手 {program} 退出码非 0: {:?}",
            output.status.code()
        );
        return None;
    }

    #[derive(Deserialize)]
    struct HelperResult {
        #[serde(rename = "Username")]
        username: String,
        #[serde(rename = "Secret")]
        secret: String,
    }

    let parsed: HelperResult = serde_json::from_slice(&output.stdout).ok()?;
    if parsed.username.is_empty() && parsed.secret.is_empty() {
        None
    } else {
        Some((parsed.username, parsed.secret))
    }
}

/// 尝试解析对应镜像仓库的认证凭据，并返回 base64 格式的 Header 字符串。
pub async fn resolve_auth_header(
    image: &str,
    explicit_auth: Option<&ContainerRegistryAuth>,
) -> Option<String> {
    if let Some(auth) = explicit_auth {
        return encode_auth_header(auth).ok();
    }
    let home = home_dir()?;
    let mut paths = Vec::with_capacity(3);
    paths.push(home.join(".docker").join("config.json"));
    paths.push(home.join(".config").join("containers").join("auth.json"));
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        paths.push(PathBuf::from(runtime).join("containers").join("auth.json"));
    }
    for path in paths {
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        let Ok(config) = serde_json::from_str::<DockerConfigFile>(&content) else {
            continue;
        };
        if let Some(header) = header_from_config(&config, image).await {
            return Some(header);
        }
    }
    None
}

/// 从一份 Docker/Podman 认证配置中解析目标镜像的 Header。
async fn header_from_config(config: &DockerConfigFile, image: &str) -> Option<String> {
    let host = extract_registry_host(image);
    let entry = config.auths.get(&host).or_else(|| {
        let stripped = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        config.auths.iter().find_map(|(key, value)| {
            let key_stripped = key
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            (key_stripped == stripped).then_some(value)
        })
    });

    if let Some(entry) = entry {
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
                if let Ok(text) = String::from_utf8(decoded) {
                    if let Some((user, pass)) = text.split_once(':') {
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
    }

    let stripped_host = host
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let helper = config
        .cred_helpers
        .get(&host)
        .or_else(|| config.cred_helpers.get(stripped_host))
        .or(config.creds_store.as_ref());
    if let Some(helper_name) = helper {
        if let Some((username, secret)) = query_credential_helper(helper_name, &host).await {
            let payload = DockerAuthPayload {
                username: &username,
                password: &secret,
                serveraddress: Some(&host),
                identitytoken: None,
            };
            if let Ok(json) = serde_json::to_string(&payload) {
                return Some(BASE64.encode(json));
            }
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
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

    #[tokio::test]
    async fn auths_entry_resolves_registry_credentials() {
        let mut config = DockerConfigFile::default();
        config.auths.insert(
            "registry.example.com".to_string(),
            DockerConfigAuthEntry {
                auth: Some(BASE64.encode("user:secret")),
                identitytoken: None,
            },
        );
        let header = header_from_config(&config, "registry.example.com/team/app:1.0")
            .await
            .expect("必须解析出认证头");
        let decoded = BASE64.decode(header).expect("认证头必须可解码");
        let json: serde_json::Value = serde_json::from_slice(&decoded).expect("必须是 JSON");
        assert_eq!(json["username"], "user");
        assert_eq!(json["password"], "secret");
        assert_eq!(json["serveraddress"], "registry.example.com");
    }

    #[test]
    fn test_docker_config_file_deserialization_with_helpers() {
        let json_data = r#"{
            "auths": {
                "https://index.docker.io/v1/": {}
            },
            "credsStore": "desktop",
            "credHelpers": {
                "gcr.io": "gcr"
            }
        }"#;
        let config: DockerConfigFile =
            serde_json::from_str(json_data).expect("反序列化 Docker 配置文件");
        assert_eq!(config.creds_store.as_deref(), Some("desktop"));
        assert_eq!(
            config.cred_helpers.get("gcr.io").map(|s| s.as_str()),
            Some("gcr")
        );
        assert!(config.auths.contains_key("https://index.docker.io/v1/"));
    }
}
