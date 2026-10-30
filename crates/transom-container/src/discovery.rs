//! 引擎发现：把「本机可能有哪些守护进程」变成一个可探测的候选清单。
//!
//! 设计原理（Why）：
//! - 发现顺序优先环境变量，其次平台已知 socket，最后才是 CLI 兜底；
//! - 候选在探测前先做存在性检查，避免每次刷新都 fork 一堆子进程；
//! - 平台差异集中在本模块，HTTP 客户端与业务层不需要知道 Windows 命名
//!   管道或 WSL 发行版的存在。

use crate::error::ContainerError;
use crate::transport::EngineTransport;
use std::path::PathBuf;
use transom_core::{ContainerEndpoint, ContainerEngineKind};

/// 一个待探测的引擎候选。
#[derive(Debug, Clone)]
pub struct EngineCandidate {
    pub kind: ContainerEngineKind,
    pub endpoint: ContainerEndpoint,
    pub transport: Option<EngineTransport>,
    /// CLI 兜底：程序与固定前缀参数。
    pub cli: Option<CliSpec>,
}

/// CLI 兜底的命令形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliSpec {
    pub program: String,
    pub prefix: Vec<String>,
}

impl CliSpec {
    /// 完整命令描述，仅用于日志与错误上下文。
    pub fn describe(&self, args: &[String]) -> String {
        let mut parts = Vec::with_capacity(self.prefix.len() + args.len() + 1);
        parts.push(self.program.clone());
        parts.extend(self.prefix.iter().cloned());
        parts.extend(args.iter().cloned());
        parts.join(" ")
    }
}

/// 生成当前平台的候选清单。
///
/// `explicit` 非 `Auto` 时只返回该端点；`Auto` 时按环境变量、已知路径与
/// CLI 的顺序合并，并按端点去重。
pub fn candidates(kind: ContainerEngineKind, explicit: &ContainerEndpoint) -> Vec<EngineCandidate> {
    if !explicit.is_auto() {
        return explicit_candidate(kind, explicit).into_iter().collect();
    }
    let mut candidates = Vec::with_capacity(8);
    if let Some(host) = environment_endpoint(kind)
        && let Some(candidate) = explicit_candidate(kind, &host)
    {
        candidates.push(candidate);
    }
    for path in known_unix_sockets(kind) {
        if path.exists() {
            candidates.push(EngineCandidate {
                kind,
                endpoint: ContainerEndpoint::Unix { path: path.clone() },
                transport: Some(EngineTransport::Unix { path }),
                cli: None,
            });
        }
    }
    #[cfg(windows)]
    candidates.extend(windows_candidates(kind));
    // CLI 兜底永远排在最后：只有前面都不可用时才会命中。
    if let Some(candidate) = cli_candidate(kind) {
        candidates.push(candidate);
    }
    dedupe(candidates)
}

/// 显式端点的候选构造；CLI 类端点在这里转成命令形态。
fn explicit_candidate(
    kind: ContainerEngineKind,
    endpoint: &ContainerEndpoint,
) -> Option<EngineCandidate> {
    let (transport, cli) = match endpoint {
        ContainerEndpoint::Auto => (None, None),
        ContainerEndpoint::Unix { path } => {
            (Some(EngineTransport::Unix { path: path.clone() }), None)
        }
        ContainerEndpoint::NamedPipe { path } => (
            Some(EngineTransport::NamedPipe { path: path.clone() }),
            None,
        ),
        ContainerEndpoint::Tcp { url } => (Some(EngineTransport::Tcp { url: url.clone() }), None),
        ContainerEndpoint::Ssh { .. } => (None, None),
        ContainerEndpoint::Wsl { distro } => (
            None,
            Some(CliSpec {
                program: "wsl.exe".to_string(),
                prefix: vec![
                    "-d".to_string(),
                    distro.clone(),
                    "-e".to_string(),
                    kind.cli_program().to_string(),
                ],
            }),
        ),
        ContainerEndpoint::Wslc => (None, Some(wslc_spec())),
        ContainerEndpoint::Cli { program, args } => (
            None,
            Some(CliSpec {
                program: program.clone(),
                prefix: args.clone(),
            }),
        ),
    };
    Some(EngineCandidate {
        kind,
        endpoint: endpoint.clone(),
        transport,
        cli,
    })
}

/// 平台已知的 Unix socket 路径。
fn known_unix_sockets(kind: ContainerEngineKind) -> Vec<PathBuf> {
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

/// 读取环境变量端点：Docker 读 `DOCKER_HOST`，Podman 读 `CONTAINER_HOST`。
fn environment_endpoint(kind: ContainerEngineKind) -> Option<ContainerEndpoint> {
    let key = match kind {
        ContainerEngineKind::Docker => "DOCKER_HOST",
        ContainerEngineKind::Podman => "CONTAINER_HOST",
    };
    let value = std::env::var(key).ok()?;
    parse_endpoint(&value)
}

/// 解析常见的端点字符串。
pub fn parse_endpoint(value: &str) -> Option<ContainerEndpoint> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(path) = value.strip_prefix("unix://") {
        return Some(ContainerEndpoint::Unix {
            path: PathBuf::from(path),
        });
    }
    if let Some(path) = value.strip_prefix("npipe://") {
        return Some(ContainerEndpoint::NamedPipe {
            path: path.to_string(),
        });
    }
    if value.starts_with("tcp://") || value.starts_with("http://") || value.starts_with("https://")
    {
        return Some(ContainerEndpoint::Tcp {
            url: value.to_string(),
        });
    }
    if value.starts_with("ssh://") {
        return Some(ContainerEndpoint::Ssh {
            target: value.trim_start_matches("ssh://").to_string(),
        });
    }
    if value.starts_with('/') || value.starts_with("~/") {
        return Some(ContainerEndpoint::Unix {
            path: PathBuf::from(value),
        });
    }
    None
}

/// CLI 兜底候选：只有可执行文件存在时才加入，避免无意义的探测。
fn cli_candidate(kind: ContainerEngineKind) -> Option<EngineCandidate> {
    let program = kind.cli_program();
    let path = find_in_path(program)?;
    Some(EngineCandidate {
        kind,
        endpoint: ContainerEndpoint::Cli {
            program: path.to_string_lossy().to_string(),
            args: Vec::new(),
        },
        transport: None,
        cli: Some(CliSpec {
            program: path.to_string_lossy().to_string(),
            prefix: Vec::new(),
        }),
    })
}

/// 在 PATH 中查找可执行文件。
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for suffix in ["exe", "cmd", "bat"] {
                let candidate = directory.join(format!("{program}.{suffix}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn is_executable(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// WSLC 的 CLI 形态：优先 `wslc.exe`，部分发行版把别名装成 `container.exe`。
fn wslc_spec() -> CliSpec {
    let program = find_in_path("wslc")
        .or_else(|| find_in_path("container"))
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "wslc.exe".to_string());
    CliSpec {
        program,
        prefix: Vec::new(),
    }
}

#[cfg(windows)]
fn windows_candidates(kind: ContainerEngineKind) -> Vec<EngineCandidate> {
    use std::path::Path;
    let mut candidates = Vec::with_capacity(4);
    if kind == ContainerEngineKind::Docker {
        for pipe in [
            r"//./pipe/docker_engine",
            r"//./pipe/dockerDesktopLinuxEngine",
        ] {
            candidates.push(EngineCandidate {
                kind,
                endpoint: ContainerEndpoint::NamedPipe {
                    path: pipe.to_string(),
                },
                transport: Some(EngineTransport::NamedPipe {
                    path: pipe.to_string(),
                }),
                cli: None,
            });
        }
    }
    // WSLC 与 WSL 发行版枚举依赖子进程，放在显式发现或用户选择时进行；
    // 这里只做低成本的可执行文件探测。
    if kind == ContainerEngineKind::Docker && find_in_path("wslc.exe").is_some() {
        candidates.push(EngineCandidate {
            kind,
            endpoint: ContainerEndpoint::Wslc,
            transport: None,
            cli: Some(wslc_spec()),
        });
    }
    let _ = Path::new(".");
    candidates
}

fn dedupe(candidates: Vec<EngineCandidate>) -> Vec<EngineCandidate> {
    let mut seen = std::collections::HashSet::with_capacity(candidates.len());
    let mut unique = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let key = (candidate.kind, endpoint_key(&candidate.endpoint));
        if seen.insert(key) {
            unique.push(candidate);
        }
    }
    unique
}

/// 去重键：unix socket 按解析后的真实路径归一。
///
/// 设计原理（Why）：`/run` 通常是 `/var/run` 的符号链接，真机上两个已知路径
/// 都存在并指向同一个守护进程；不归一就会在界面里列出两条完全等价的引擎，
/// 用户无法分辨也删不掉重复项。归一失败（无权限、路径不存在）时退回原描述，
/// 宁可多列一项也不漏掉一个可用引擎。
fn endpoint_key(endpoint: &ContainerEndpoint) -> String {
    if let ContainerEndpoint::Unix { path } = endpoint {
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        return format!("unix://{}", resolved.display());
    }
    endpoint.describe()
}

/// 显式端点是否在当前平台可用；用于给会话表单一个提前拒绝的机会。
pub fn validate_endpoint(endpoint: &ContainerEndpoint) -> Result<(), ContainerError> {
    match endpoint {
        ContainerEndpoint::Ssh { .. } => Err(ContainerError::Unsupported {
            operation: "SSH 引擎端点（计划在 P1 提供）".to_string(),
        }),
        ContainerEndpoint::NamedPipe { .. } if !cfg!(windows) => Err(ContainerError::Unsupported {
            operation: "命名管道仅支持 Windows".to_string(),
        }),
        ContainerEndpoint::Wsl { .. } | ContainerEndpoint::Wslc if !cfg!(windows) => {
            Err(ContainerError::Unsupported {
                operation: "WSL/WSLC 端点仅支持 Windows".to_string(),
            })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{endpoint_key, parse_endpoint};
    use std::path::PathBuf;
    use transom_core::ContainerEndpoint;
    #[test]
    fn parses_common_endpoint_forms() {
        assert_eq!(
            parse_endpoint("unix:///var/run/docker.sock"),
            Some(ContainerEndpoint::Unix {
                path: PathBuf::from("/var/run/docker.sock")
            })
        );
        assert_eq!(
            parse_endpoint("npipe:////./pipe/docker_engine"),
            Some(ContainerEndpoint::NamedPipe {
                path: "//./pipe/docker_engine".to_string()
            })
        );
        assert_eq!(
            parse_endpoint("tcp://127.0.0.1:2375"),
            Some(ContainerEndpoint::Tcp {
                url: "tcp://127.0.0.1:2375".to_string()
            })
        );
        assert!(matches!(
            parse_endpoint("ssh://user@host"),
            Some(ContainerEndpoint::Ssh { .. })
        ));
        assert_eq!(parse_endpoint("   "), None);
        assert_eq!(parse_endpoint("relative/path"), None);
    }

    /// 真机回归：`/run/docker.sock` 与 `/var/run/docker.sock` 是同一个文件，
    /// 去重键必须归一到真实路径，否则界面会出现两条等价引擎。
    #[cfg(unix)]
    #[test]
    fn symlinked_socket_paths_share_one_key() {
        let dir = tempfile::tempdir().expect("临时目录必须可建");
        let real = dir.path().join("docker.sock");
        std::fs::write(&real, b"").expect("占位文件必须可写");
        let link = dir.path().join("run.sock");
        std::os::unix::fs::symlink(&real, &link).expect("符号链接必须可建");
        assert_eq!(
            endpoint_key(&ContainerEndpoint::Unix { path: real.clone() }),
            endpoint_key(&ContainerEndpoint::Unix { path: link })
        );
        // 解析不了的路径退回原描述：宁可多列一项，也不能把可用引擎藏起来。
        let missing = dir.path().join("missing.sock");
        assert_eq!(
            endpoint_key(&ContainerEndpoint::Unix {
                path: missing.clone()
            }),
            format!("unix://{}", missing.display())
        );
    }
}
