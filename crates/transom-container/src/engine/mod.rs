//! 引擎实例：把「探测结果」与「一次具体操作」收敛到同一个对象。
//!
//! 一个 [`Engine`] 要么走 HTTP（Docker/Podman 兼容 REST），要么走 CLI 兜底；
//! 业务层只调用 `list_containers` 之类的方法，不关心底层通道。
//!
//! 操作面按资源域拆到 operations.rs，发现/探测入口拆到 discovery.rs；
//! 共享操作通过 dispatch! 宏生成唯一分派点。

use crate::api::DockerApi;
use crate::cli::CliBackend;
use crate::discovery::EngineCandidate;
use crate::error::ContainerError;
use transom_core::{ContainerEngineInfo, ContainerEngineRef};

#[derive(Debug, Clone)]
enum EngineBackend {
    Http(DockerApi),
    Cli(CliBackend),
}

/// 一个已确认可用的容器引擎。
#[derive(Debug, Clone)]
pub struct Engine {
    pub info: ContainerEngineInfo,
    backend: EngineBackend,
}

/// 将同签名的操作分发给当前后端。
///
/// 设计原理（Why）：HTTP 与 CLI 后端在共享能力上的方法签名一致，用声明式
/// 分派宏生成唯一分派点，避免为每个操作手写两个分支后出现参数漏改或分支
/// 漂移。仅用于两个后端都支持且签名一致的操作；能力差异（仅 HTTP/仅 CLI）
/// 仍保留显式分支，让能力矩阵在代码里一眼可见。
macro_rules! dispatch {
    ($self:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$self.backend {
            EngineBackend::Http(api) => api.$method($($arg),*).await,
            EngineBackend::Cli(cli) => cli.$method($($arg),*).await,
        }
    };
}

mod discovery;
mod operations;

pub use discovery::{discover_all, probe_explicit};

impl Engine {
    /// 探测候选并构造引擎。
    ///
    /// # Errors
    /// HTTP 端点不可达、版本响应异常或 CLI 不可执行时返回对应错误。
    pub async fn probe(candidate: EngineCandidate) -> Result<Self, ContainerError> {
        if let Some(transport) = candidate.transport.clone() {
            let api = DockerApi::new(transport, None);
            match api.probe(candidate.kind).await {
                Ok(probed) => {
                    let api = api.with_api_version(&probed.api_version);
                    let info = ContainerEngineInfo {
                        engine: ContainerEngineRef::explicit(candidate.kind, candidate.endpoint),
                        version: if probed.version.is_empty() {
                            "未知版本".to_string()
                        } else {
                            probed.version
                        },
                        api_version: probed.api_version,
                        platform: probed.platform,
                        cli_backend: false,
                    };
                    return Ok(Self {
                        info,
                        backend: EngineBackend::Http(api),
                    });
                }
                Err(error) => {
                    if candidate.cli.is_none() {
                        return Err(error);
                    }
                    log::warn!(
                        "传输通道探测失败，降级到 CLI 兜底，端点: {}，原因: {error}",
                        candidate.endpoint.describe()
                    );
                }
            }
        }
        if let Some(spec) = candidate.cli {
            let cli = CliBackend::new(candidate.kind, spec);
            let (version, api_version) = cli.probe().await?;
            let info = ContainerEngineInfo {
                engine: ContainerEngineRef::explicit(candidate.kind, candidate.endpoint),
                version,
                api_version,
                platform: format!("{} CLI", candidate.kind.label()),
                cli_backend: true,
            };
            return Ok(Self {
                info,
                backend: EngineBackend::Cli(cli),
            });
        }
        Err(ContainerError::Unsupported {
            operation: "候选端点既没有传输通道也没有 CLI".to_string(),
        })
    }

    /// HTTP 客户端；CLI 后端返回不支持。
    fn api(&self) -> Result<&DockerApi, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => Ok(api),
            EngineBackend::Cli(_) => Err(ContainerError::Unsupported {
                operation: "当前引擎走 CLI 后端，不支持该操作".to_string(),
            }),
        }
    }

    fn cli(&self) -> Option<&CliBackend> {
        match &self.backend {
            EngineBackend::Cli(cli) => Some(cli),
            EngineBackend::Http(_) => None,
        }
    }

    /// 该引擎是否走 CLI 兜底（能力更少）。
    pub fn cli_backend(&self) -> bool {
        self.info.cli_backend
    }

    pub fn http(&self) -> Result<&DockerApi, ContainerError> {
        self.api()
    }

    /// CLI 入口：exec 与日志跟随需要它。
    pub fn cli_backend_ref(&self) -> Option<&CliBackend> {
        self.cli()
    }
}
