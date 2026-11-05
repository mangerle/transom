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
use transom_core::{ContainerEndpoint, ContainerEngineKind, TlsConfig};

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
    // 自动探测 Docker 时，引入系统已配置的 Docker Contexts（如远程 SSH、Colima 等）
    if kind == ContainerEngineKind::Docker {
        candidates.extend(crate::docker_context::discover_context_candidates());
    }
    if kind == ContainerEngineKind::Podman {
        candidates.extend(podman_machine_candidates());
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
pub(crate) fn explicit_candidate(
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
        ContainerEndpoint::Tcp { url, tls } => (
            Some(EngineTransport::Tcp {
                url: url.clone(),
                tls: tls.clone(),
            }),
            None,
        ),
        ContainerEndpoint::Ssh {
            host,
            port,
            user,
            identity_file,
        } => {
            let args = build_ssh_dial_stdio_args(
                kind,
                host,
                *port,
                user.as_deref(),
                identity_file.as_deref(),
            );
            (
                Some(EngineTransport::Stdio {
                    program: "ssh".to_string(),
                    args,
                }),
                None,
            )
        }
        ContainerEndpoint::Wsl { distro } => {
            let transport_args = vec![
                "-d".to_string(),
                distro.clone(),
                "-e".to_string(),
                kind.cli_program().to_string(),
                "system".to_string(),
                "dial-stdio".to_string(),
            ];
            let cli_prefix = vec![
                "-d".to_string(),
                distro.clone(),
                "-e".to_string(),
                kind.cli_program().to_string(),
            ];
            (
                Some(EngineTransport::Stdio {
                    program: "wsl.exe".to_string(),
                    args: transport_args,
                }),
                Some(CliSpec {
                    program: "wsl.exe".to_string(),
                    prefix: cli_prefix,
                }),
            )
        }
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

/// 通过 `podman machine inspect` 枚举 machine 暴露的 socket。
///
/// 设计原理（Why）：macOS/Windows 的 Podman 通常跑在虚拟机里，socket 路径由
/// machine 动态分配；只靠固定路径会漏掉用户已经启动的 machine。
fn podman_machine_candidates() -> Vec<EngineCandidate> {
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
    let mut child = std::process::Command::new("podman")
        .args(["machine", "inspect", "--format", "json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            log::debug!("podman machine inspect 超时，跳过 machine 发现");
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let output = child.wait_with_output().ok()?;
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
fn environment_endpoint(kind: ContainerEngineKind) -> Option<ContainerEndpoint> {
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
            // https 明确要求 TLS；证书路径由环境变量或默认目录补充。
            tls: value.starts_with("https://").then(TlsConfig::default),
        });
    }
    if let Some(rest) = value.strip_prefix("ssh://") {
        return Some(parse_ssh_endpoint(rest));
    }
    if value.starts_with('/') || value.starts_with("~/") {
        return Some(ContainerEndpoint::Unix {
            path: PathBuf::from(value),
        });
    }
    None
}

/// 解析 SSH 地址字符串（不含 `ssh://` 前缀）。
///
/// 支持格式：`host`、`host:port`、`user@host`、`user@host:port`。
/// 端口必须是合法的 u16，否则整个字符串按 host 处理。
fn parse_ssh_endpoint(s: &str) -> ContainerEndpoint {
    // 先分离 user 部分
    let (user, rest) = if let Some(idx) = s.find('@') {
        let u = &s[..idx];
        (
            if u.is_empty() {
                None
            } else {
                Some(u.to_string())
            },
            &s[idx + 1..],
        )
    } else {
        (None, s)
    };
    // 再分离 host 和 port
    let (host, port) = if let Some((h, p)) = rest.rsplit_once(':') {
        if let Ok(port_num) = p.parse::<u16>() {
            (h.to_string(), Some(port_num))
        } else {
            (rest.to_string(), None)
        }
    } else {
        (rest.to_string(), None)
    };
    ContainerEndpoint::Ssh {
        host,
        port,
        user,
        identity_file: None,
    }
}

/// 构造通过 SSH 远程执行 `docker system dial-stdio` 的子进程参数。
fn build_ssh_dial_stdio_args(
    kind: ContainerEngineKind,
    host: &str,
    port: Option<u16>,
    user: Option<&str>,
    identity_file: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-T".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        // 不在首次连接时静默信任未知主机密钥；用户应先用 SSH 会话确认指纹，
        // 或自行把主机写入 known_hosts，避免中间人攻击。
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
    ];
    if let Some(port) = port {
        args.push("-p".to_string());
        args.push(port.to_string());
    }
    if let Some(identity) = identity_file {
        args.push("-i".to_string());
        args.push(identity.to_string());
    }
    let target = if let Some(user) = user {
        format!("{user}@{host}")
    } else {
        host.to_string()
    };
    args.push(target);
    args.push(kind.cli_program().to_string());
    args.push("system".to_string());
    args.push("dial-stdio".to_string());
    args
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
    use super::{endpoint_key, parse_endpoint, parse_podman_machine_inspect, parse_ssh_endpoint};
    use std::path::PathBuf;
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
                url: "tcp://127.0.0.1:2375".to_string(),
                tls: None,
            })
        );
        assert!(matches!(
            parse_endpoint("ssh://user@host"),
            Some(ContainerEndpoint::Ssh { .. })
        ));
        assert!(matches!(
            parse_endpoint("https://docker.example.com:2376"),
            Some(ContainerEndpoint::Tcp { tls: Some(_), .. })
        ));
        assert!(matches!(
            parse_endpoint("tcp://127.0.0.1:2375"),
            Some(ContainerEndpoint::Tcp { tls: None, .. })
        ));
        assert_eq!(parse_endpoint("   "), None);
        assert_eq!(parse_endpoint("relative/path"), None);
    }

    #[test]
    fn parses_ssh_endpoint_formats() {
        // host only
        let ep = parse_ssh_endpoint("myhost.example.com");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, port: None, user: None, .. } if host == "myhost.example.com")
        );

        // user@host
        let ep = parse_ssh_endpoint("admin@192.168.1.100");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, user: Some(u), port: None, .. } if host == "192.168.1.100" && u == "admin")
        );

        // host:port
        let ep = parse_ssh_endpoint("myhost:2222");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, port: Some(2222), user: None, .. } if host == "myhost")
        );

        // user@host:port
        let ep = parse_ssh_endpoint("root@10.0.0.1:22");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, port: Some(22), user: Some(u), .. } if host == "10.0.0.1" && u == "root")
        );
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

    #[test]
    fn ssh_endpoint_constructs_stdio_transport() {
        use crate::transport::EngineTransport;
        use transom_core::ContainerEngineKind;

        let ep = ContainerEndpoint::Ssh {
            host: "example.com".to_string(),
            port: Some(2222),
            user: Some("ubuntu".to_string()),
            identity_file: Some("/home/ubuntu/.ssh/id_ed25519".to_string()),
        };
        let cands = super::candidates(ContainerEngineKind::Docker, &ep);
        assert_eq!(cands.len(), 1);
        let cand = &cands[0];
        assert_eq!(cand.kind, ContainerEngineKind::Docker);
        let Some(EngineTransport::Stdio { program, args }) = &cand.transport else {
            panic!("必须构造出 Stdio 通道");
        };
        assert_eq!(program, "ssh");
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"/home/ubuntu/.ssh/id_ed25519".to_string()));
        assert!(args.contains(&"ubuntu@example.com".to_string()));
        assert!(args.contains(&"dial-stdio".to_string()));
    }

    #[test]
    fn wsl_endpoint_constructs_stdio_transport_and_cli() {
        use crate::transport::EngineTransport;
        use transom_core::ContainerEngineKind;

        let ep = ContainerEndpoint::Wsl {
            distro: "Ubuntu-24.04".to_string(),
        };
        let cands = super::candidates(ContainerEngineKind::Docker, &ep);
        assert_eq!(cands.len(), 1);
        let cand = &cands[0];
        let Some(EngineTransport::Stdio { program, args }) = &cand.transport else {
            panic!("WSL 必须包含 Stdio 通道");
        };
        assert_eq!(program, "wsl.exe");
        assert_eq!(
            args,
            &["-d", "Ubuntu-24.04", "-e", "docker", "system", "dial-stdio"]
        );
        let Some(cli) = &cand.cli else {
            panic!("WSL 必须保留 CLI 兜底配置");
        };
        assert_eq!(cli.program, "wsl.exe");
        assert_eq!(cli.prefix, &["-d", "Ubuntu-24.04", "-e", "docker"]);
    }
}
