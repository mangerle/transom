//! 引擎发现：把「本机可能有哪些守护进程」变成一个可探测的候选清单。
//!
//! 设计原理（Why）：
//! - 发现顺序优先环境变量，其次平台已知 socket，最后才是 CLI 兜底；
//! - 候选在探测前先做存在性检查，避免每次刷新都 fork 一堆子进程；
//! - 平台差异集中在 discovery/ 下，HTTP 客户端与业务层不需要知道 Windows
//!   命名管道或 WSL 发行版的存在。

mod cli;
mod parse;
mod unix;
#[cfg(windows)]
mod windows;

pub use cli::find_in_path;
pub use parse::parse_endpoint;

use crate::error::ContainerError;
use crate::transport::EngineTransport;
use cli::{cli_candidate, wslc_spec};
use transom_core::{ContainerEndpoint, ContainerEngineKind};
use unix::{environment_endpoint, known_unix_sockets, podman_machine_candidates};
#[cfg(windows)]
use windows::windows_candidates;

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
            password,
            use_sudo,
            sudo_password,
        } => {
            let command = format!("{} system dial-stdio", kind.cli_program());
            (
                Some(EngineTransport::Ssh {
                    host: host.clone(),
                    port: port.unwrap_or(22),
                    user: user.clone().unwrap_or_else(|| {
                        std::env::var("USER")
                            .or_else(|_| std::env::var("USERNAME"))
                            .unwrap_or_else(|_| "root".to_string())
                    }),
                    password: password.clone(),
                    identity_file: identity_file.clone(),
                    use_sudo: *use_sudo,
                    sudo_password: sudo_password.clone(),
                    command,
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
    use super::*;

    #[test]
    fn ssh_endpoint_constructs_ssh_transport() {
        use crate::transport::EngineTransport;
        use transom_core::ContainerEngineKind;

        let ep = ContainerEndpoint::Ssh {
            host: "example.com".to_string(),
            port: Some(2222),
            user: Some("ubuntu".to_string()),
            identity_file: Some("/home/ubuntu/.ssh/id_ed25519".to_string()),
            password: Some("mypassword".to_string()),
            use_sudo: true,
            sudo_password: Some("sudopass".to_string()),
        };
        let cands = super::candidates(ContainerEngineKind::Docker, &ep);
        assert_eq!(cands.len(), 1);
        let cand = &cands[0];
        assert_eq!(cand.kind, ContainerEngineKind::Docker);
        let Some(EngineTransport::Ssh {
            host,
            port,
            user,
            password,
            identity_file,
            use_sudo,
            sudo_password,
            command,
        }) = &cand.transport
        else {
            panic!("必须构造出 Ssh 通道");
        };
        assert_eq!(host, "example.com");
        assert_eq!(*port, 2222);
        assert_eq!(user, "ubuntu");
        assert_eq!(password.as_deref(), Some("mypassword"));
        assert_eq!(
            identity_file.as_deref(),
            Some("/home/ubuntu/.ssh/id_ed25519")
        );
        assert!(*use_sudo);
        assert_eq!(sudo_password.as_deref(), Some("sudopass"));
        assert_eq!(command, "docker system dial-stdio");
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
