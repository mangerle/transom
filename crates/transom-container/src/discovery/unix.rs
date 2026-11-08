//! unix 相关的引擎候选发现。

use super::parse::parse_endpoint;
use super::{EngineCandidate, explicit_candidate};
use std::path::PathBuf;
use transom_core::{ContainerEndpoint, ContainerEngineKind, TlsConfig};

/// 平台已知的 Unix socket 路径。
pub(super) fn known_unix_sockets(kind: ContainerEngineKind) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(4);
    if cfg!(unix) {
        let xdg_runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        match kind {
            ContainerEngineKind::Docker => {
                paths.push(PathBuf::from("/var/run/docker.sock"));
                paths.push(PathBuf::from("/run/docker.sock"));
                if let Some(runtime) = &xdg_runtime {
                    paths.push(runtime.join("docker.sock"));
                }
                if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
                    paths.push(home.join(".docker/run/docker.sock"));
                }
            }
            ContainerEngineKind::Podman => {
                paths.push(PathBuf::from("/run/podman/podman.sock"));
                if let Some(runtime) = &xdg_runtime {
                    paths.push(runtime.join("podman/podman.sock"));
                }
                if let Ok(uid) = std::env::var("UID") {
                    paths.push(PathBuf::from(format!("/run/user/{uid}/podman/podman.sock")));
                }
            }
        }
    }
    paths
}

/// 通过 `podman machine inspect` 枚举 machine 暴露的 socket。
///
/// 设计原理（Why）：macOS/Windows 的 Podman 通常跑在虚拟机里，socket 路径由
/// machine 动态分配；只靠固定路径会漏掉用户已经启动的 machine。
pub(super) fn podman_machine_candidates() -> Vec<EngineCandidate> {
    let output = match run_podman_machine_inspect() {
        Some(output) => output,
        None => return Vec::new(),
    };
    parse_podman_machine_inspect(&output)
        .into_iter()
        .filter_map(|endpoint| explicit_candidate(ContainerEngineKind::Podman, &endpoint))
        .collect()
}

/// 执行 `podman machine inspect --format json`，带 5 秒上限。
fn run_podman_machine_inspect() -> Option<Vec<u8>> {
    let mut command = std::process::Command::new("podman");
    command.args(["machine", "inspect", "--format", "json"]);
    let output = crate::cli::run_grouped_output(&mut command, std::time::Duration::from_secs(5))
        .map_err(|error| log::debug!("podman machine inspect 失败，跳过 machine 发现: {error}"))
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// 解析 podman machine inspect 的 JSON，兼容单机对象与数组两种形态。
fn parse_podman_machine_inspect(bytes: &[u8]) -> Vec<ContainerEndpoint> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let machines: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(_) => vec![&value],
        _ => return Vec::new(),
    };
    let mut endpoints = Vec::with_capacity(machines.len());
    for machine in machines {
        let path = machine
            .pointer("/ConnectionInfo/PodmanSocket/Path")
            .and_then(|value| value.as_str())
            .or_else(|| {
                machine
                    .pointer("/ConnectionInfo/Socket/Path")
                    .and_then(|value| value.as_str())
            });
        let Some(path) = path.filter(|path| !path.trim().is_empty()) else {
            continue;
        };
        let endpoint = if path.starts_with("//./pipe") || path.starts_with("\\.") {
            ContainerEndpoint::NamedPipe {
                path: path.to_string(),
            }
        } else {
            ContainerEndpoint::Unix {
                path: PathBuf::from(path),
            }
        };
        endpoints.push(endpoint);
    }
    endpoints
}

/// 读取环境变量端点：Docker 读 `DOCKER_HOST`，Podman 读 `CONTAINER_HOST`。
pub(super) fn environment_endpoint(kind: ContainerEngineKind) -> Option<ContainerEndpoint> {
    let key = match kind {
        ContainerEngineKind::Docker => "DOCKER_HOST",
        ContainerEngineKind::Podman => "CONTAINER_HOST",
    };
    let value = std::env::var(key).ok()?;
    let mut endpoint = parse_endpoint(&value)?;
    if let ContainerEndpoint::Tcp { url, tls } = &mut endpoint
        && (url.starts_with("https://") || tls_requested(kind))
    {
        *tls = Some(tls_config_from_environment(kind));
    }
    Some(endpoint)
}

/// 环境变量是否显式要求 TLS。
fn tls_requested(kind: ContainerEngineKind) -> bool {
    let key = match kind {
        ContainerEngineKind::Docker => "DOCKER_TLS_VERIFY",
        ContainerEngineKind::Podman => "CONTAINER_TLS_VERIFY",
    };
    std::env::var(key)
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false)
}

/// 从 `DOCKER_CERT_PATH`（Podman 为 `CONTAINER_CERT_PATH`）或 `~/.docker`
/// 读取三件套路径；文件不存在时留空，由 rustls 使用公共根证书。
fn tls_config_from_environment(kind: ContainerEngineKind) -> TlsConfig {
    let cert_key = match kind {
        ContainerEngineKind::Docker => "DOCKER_CERT_PATH",
        ContainerEngineKind::Podman => "CONTAINER_CERT_PATH",
    };
    let cert_dir = std::env::var_os(cert_key)
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".docker")));
    let path_for = |name: &str| {
        cert_dir
            .as_ref()
            .map(|dir| dir.join(name))
            .filter(|path| path.is_file())
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    };
    TlsConfig {
        ca_cert: path_for("ca.pem"),
        client_cert: path_for("cert.pem"),
        client_key: path_for("key.pem"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::ContainerEndpoint;

    #[test]
    fn podman_machine_inspect_extracts_socket_paths() {
        let json = br#"[
          {"Name":"default","ConnectionInfo":{"PodmanSocket":{"Path":"/tmp/podman.sock"}}},
          {"Name":"win","ConnectionInfo":{"PodmanSocket":{"Path":"//./pipe/podman-machine-default"}}}
        ]"#;
        let endpoints = parse_podman_machine_inspect(json);
        assert_eq!(endpoints.len(), 2);
        assert!(matches!(
            &endpoints[0],
            ContainerEndpoint::Unix { path }
                if path.as_path() == std::path::Path::new("/tmp/podman.sock")
        ));
        assert!(matches!(
            &endpoints[1],
            ContainerEndpoint::NamedPipe { path } if path == "//./pipe/podman-machine-default"
        ));
    }
}
