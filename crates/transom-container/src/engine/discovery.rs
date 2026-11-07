//! 容器引擎发现与显式端点探测。

use super::Engine;
use crate::discovery;
use crate::discovery::{CliSpec, EngineCandidate};
use crate::error::ContainerError;
use crate::transport::EngineTransport;
use std::time::Duration;
use transom_core::{ContainerEndpoint, ContainerEngineKind};

/// 探测单个候选的超时：发现是交互触发的，不能让一个坏端点拖住整页。
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// 发现本机全部引擎；单个候选失败只记录原因，不影响其它候选。
///
/// 返回 `(可用引擎, 失败摘要)`。失败摘要用于界面提示「Docker 未启动」这类
/// 可操作信息，不包含凭证。
///
/// CLI 兜底不单独成项（Why）：候选清单里 CLI 永远排在最后，只有同种类没有任何
/// 可用 REST 端点时它才是唯一入口；若两者都可用就并存两条 Docker 条目，用户在
/// 界面里既分不清也删不掉，而 CLI 后端还少 resize 与归档能力。
pub async fn discover_all() -> (Vec<Engine>, Vec<String>) {
    // 候选生成会扫描文件系统并可能调用 `docker context ls`，放到阻塞线程池，
    // 避免在 tokio worker 上做同步 IO 或等待子进程。
    let candidates = match tokio::task::spawn_blocking(|| {
        let mut candidates =
            discovery::candidates(ContainerEngineKind::Docker, &ContainerEndpoint::Auto);
        candidates.extend(discovery::candidates(
            ContainerEngineKind::Podman,
            &ContainerEndpoint::Auto,
        ));
        candidates
    })
    .await
    {
        Ok(candidates) => candidates,
        Err(error) => {
            log::error!("容器引擎候选生成任务失败，原因: {error}");
            Vec::new()
        }
    };
    let mut engines = Vec::with_capacity(candidates.len());
    let mut failures = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let endpoint = candidate.endpoint.describe();
        let kind = candidate.kind;
        match tokio::time::timeout(PROBE_TIMEOUT, Engine::probe(candidate)).await {
            Ok(Ok(engine)) => {
                log::info!(
                    "发现容器引擎，种类: {kind}，端点: {endpoint}，版本: {}",
                    engine.info.version
                );
                engines.push(engine);
            }
            Ok(Err(error)) => {
                log::debug!("容器引擎候选不可用，种类: {kind}，端点: {endpoint}，原因: {error}");
                failures.push(format!("{kind}（{endpoint}）：{error}"));
            }
            Err(_) => {
                log::debug!("容器引擎候选探测超时，种类: {kind}，端点: {endpoint}");
                failures.push(format!("{kind}（{endpoint}）：探测超时"));
            }
        }
    }
    drop_cli_fallbacks(&mut engines);
    (engines, failures)
}

/// 同种类已有 REST 端点时，剔除该种类的 CLI 兜底引擎。
fn drop_cli_fallbacks(engines: &mut Vec<Engine>) {
    let http_kinds: Vec<ContainerEngineKind> = engines
        .iter()
        .filter(|engine| !engine.info.cli_backend)
        .map(|engine| engine.info.engine.kind)
        .collect();
    let before = engines.len();
    engines.retain(|engine| {
        !engine.info.cli_backend || !http_kinds.contains(&engine.info.engine.kind)
    });
    let dropped = before - engines.len();
    if dropped > 0 {
        log::debug!("容器引擎发现已剔除 {dropped} 个 CLI 兜底候选，同种类已有可用的 REST 端点");
    }
}

/// 探测一个显式端点；供「自动发现失败后用户手动指定」使用。
pub async fn probe_explicit(
    kind: ContainerEngineKind,
    endpoint: &ContainerEndpoint,
) -> Result<Engine, ContainerError> {
    discovery::validate_endpoint(endpoint)?;
    let candidate = discovery::candidates(kind, endpoint)
        .into_iter()
        .next()
        .unwrap_or_else(|| EngineCandidate {
            kind,
            endpoint: endpoint.clone(),
            transport: transport_for(endpoint),
            cli: cli_for(kind, endpoint),
        });
    tokio::time::timeout(PROBE_TIMEOUT, Engine::probe(candidate))
        .await
        .map_err(|_| ContainerError::Http {
            operation: "探测显式端点".to_string(),
            message: "探测超时".to_string(),
        })?
}

fn transport_for(endpoint: &ContainerEndpoint) -> Option<EngineTransport> {
    match endpoint {
        ContainerEndpoint::Unix { path } => Some(EngineTransport::Unix { path: path.clone() }),
        ContainerEndpoint::NamedPipe { path } => {
            Some(EngineTransport::NamedPipe { path: path.clone() })
        }
        ContainerEndpoint::Tcp { url, tls } => Some(EngineTransport::Tcp {
            url: url.clone(),
            tls: tls.clone(),
        }),
        _ => None,
    }
}

fn cli_for(kind: ContainerEngineKind, endpoint: &ContainerEndpoint) -> Option<CliSpec> {
    match endpoint {
        ContainerEndpoint::Wsl { distro } => Some(CliSpec {
            program: "wsl.exe".to_string(),
            prefix: vec![
                "-d".to_string(),
                distro.clone(),
                "-e".to_string(),
                kind.cli_program().to_string(),
            ],
        }),
        ContainerEndpoint::Wslc => Some(CliSpec {
            program: "wslc.exe".to_string(),
            prefix: Vec::new(),
        }),
        ContainerEndpoint::Cli { program, args } => Some(CliSpec {
            program: program.clone(),
            prefix: args.clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Engine, EngineBackend};
    use super::transport_for;
    use crate::api::DockerApi;
    use crate::cli::CliBackend;
    use crate::discovery::CliSpec;
    use crate::transport::EngineTransport;
    use std::path::PathBuf;
    use transom_core::{
        ContainerEndpoint, ContainerEngineInfo, ContainerEngineKind, ContainerEngineRef,
    };

    #[test]
    fn unix_endpoint_builds_unix_transport() {
        let transport = transport_for(&ContainerEndpoint::Unix {
            path: PathBuf::from("/var/run/docker.sock"),
        });
        assert!(matches!(transport, Some(EngineTransport::Unix { .. })));
        assert!(transport_for(&ContainerEndpoint::Auto).is_none());
    }

    fn stub_engine(kind: ContainerEngineKind, cli_backend: bool) -> Engine {
        let backend = if cli_backend {
            EngineBackend::Cli(CliBackend::new(
                kind,
                CliSpec {
                    program: "docker".to_string(),
                    prefix: Vec::new(),
                },
            ))
        } else {
            EngineBackend::Http(DockerApi::new(
                EngineTransport::Unix {
                    path: PathBuf::from("/nonexistent/docker.sock"),
                },
                None,
            ))
        };
        Engine {
            info: ContainerEngineInfo {
                engine: ContainerEngineRef::auto(kind),
                version: "1.0".to_string(),
                api_version: "1.52".to_string(),
                platform: "linux/amd64".to_string(),
                cli_backend,
            },
            backend,
        }
    }

    /// 真机回归：本机同时存在 socket 与 CLI 入口时，只保留 REST 引擎。
    #[test]
    fn cli_fallback_is_dropped_when_rest_engine_exists() {
        let mut engines = vec![
            stub_engine(ContainerEngineKind::Docker, false),
            stub_engine(ContainerEngineKind::Docker, true),
            stub_engine(ContainerEngineKind::Podman, true),
        ];
        super::drop_cli_fallbacks(&mut engines);
        assert_eq!(engines.len(), 2);
        assert!(!engines[0].info.cli_backend);
        // Podman 只有 CLI 命中时必须保留，否则用户彻底没有入口。
        assert_eq!(engines[1].info.engine.kind, ContainerEngineKind::Podman);
        assert!(engines[1].info.cli_backend);
    }
}
