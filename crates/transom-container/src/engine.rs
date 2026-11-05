//! 引擎实例：把「探测结果」与「一次具体操作」收敛到同一个对象。
//!
//! 一个 [`Engine`] 要么走 HTTP（Docker/Podman 兼容 REST），要么走 CLI 兜底；
//! 业务层只调用 `list_containers` 之类的方法，不关心底层通道。

use crate::api::DockerApi;
use crate::cli::CliBackend;
use crate::discovery::{self, CliSpec, EngineCandidate};
use crate::error::ContainerError;
use crate::transport::EngineTransport;
use std::time::Duration;
use transom_core::{
    ComposeProject, ContainerBuildSpec, ContainerDetails, ContainerEndpoint, ContainerEngineInfo,
    ContainerEngineKind, ContainerEngineRef, ContainerImageDetails, ContainerImageLayer,
    ContainerImageSummary, ContainerLifecycleAction, ContainerNetworkDetails,
    ContainerNetworkSummary, ContainerProcess, ContainerRegistryAuth, ContainerStats,
    ContainerSummary, ContainerUpdateSpec, ContainerVolumeDetails, ContainerVolumeSummary,
    compose_projects,
};

/// 探测单个候选的超时：发现是交互触发的，不能让一个坏端点拖住整页。
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

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

    pub async fn list_containers(&self) -> Result<Vec<ContainerSummary>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.list_containers().await,
            EngineBackend::Cli(cli) => cli.list_containers().await,
        }
    }

    /// 列出 Podman Pod；HTTP 兼容层没有 libpod 端点，当前只支持 CLI 连接。
    ///
    /// # Errors
    /// 非 Podman 引擎、HTTP 连接或 CLI 执行失败时返回错误。
    pub async fn list_pods(
        &self,
    ) -> Result<Vec<transom_core::ContainerPodSummary>, ContainerError> {
        match &self.backend {
            EngineBackend::Cli(cli) => cli.list_pods().await,
            EngineBackend::Http(_) => Err(ContainerError::Unsupported {
                operation: "Pod 管理需要 Podman CLI 连接，当前 HTTP 连接暂不支持 libpod Pod API"
                    .to_string(),
            }),
        }
    }

    /// 执行 Podman Pod 启停/重启/删除。
    ///
    /// # Errors
    /// 非 Podman 引擎、HTTP 连接或 CLI 执行失败时返回错误。
    pub async fn pod_action(
        &self,
        pod: &str,
        action: transom_core::ContainerPodAction,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Cli(cli) => cli.pod_action(pod, action).await,
            EngineBackend::Http(_) => Err(ContainerError::Unsupported {
                operation: "Pod 管理需要 Podman CLI 连接，当前 HTTP 连接暂不支持 libpod Pod API"
                    .to_string(),
            }),
        }
    }

    pub async fn list_images(&self) -> Result<Vec<ContainerImageSummary>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.list_images().await,
            EngineBackend::Cli(cli) => cli.list_images().await,
        }
    }

    pub async fn inspect_container(
        &self,
        container: &str,
    ) -> Result<ContainerDetails, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.inspect_container(container).await,
            EngineBackend::Cli(cli) => cli.inspect_container(container).await,
        }
    }

    pub async fn container_changes(
        &self,
        container: &str,
    ) -> Result<Vec<transom_core::ContainerFileSystemChange>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.container_changes(container).await,
            EngineBackend::Cli(cli) => cli.container_changes(container).await,
        }
    }

    pub async fn system_df(
        &self,
    ) -> Result<transom_core::ContainerSystemDiskUsage, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.system_df().await,
            EngineBackend::Cli(cli) => cli.system_df().await,
        }
    }

    pub async fn system_prune(
        &self,
        all: bool,
        volumes: bool,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.system_prune(all, volumes).await,
            EngineBackend::Cli(cli) => cli.system_prune(all, volumes).await,
        }
    }

    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<ContainerImageDetails, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.inspect_image(image).await,
            EngineBackend::Cli(cli) => cli.inspect_image(image).await,
        }
    }

    pub async fn image_history(
        &self,
        image: &str,
    ) -> Result<Vec<ContainerImageLayer>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.image_history(image).await,
            EngineBackend::Cli(cli) => cli.image_history(image).await,
        }
    }

    pub async fn tag_image(
        &self,
        image: &str,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.tag_image(image, repo, tag).await,
            EngineBackend::Cli(cli) => cli.tag_image(image, repo, tag).await,
        }
    }

    pub async fn list_volumes(&self) -> Result<Vec<ContainerVolumeSummary>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.list_volumes().await,
            EngineBackend::Cli(cli) => cli.list_volumes().await,
        }
    }

    pub async fn inspect_volume(
        &self,
        name: &str,
    ) -> Result<ContainerVolumeDetails, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.inspect_volume(name).await,
            EngineBackend::Cli(cli) => cli.inspect_volume(name).await,
        }
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.create_volume(name, driver).await,
            EngineBackend::Cli(cli) => cli.create_volume(name, driver).await,
        }
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.remove_volume(name, force).await,
            EngineBackend::Cli(cli) => cli.remove_volume(name, force).await,
        }
    }

    /// 清理悬空镜像；与卷一样，REST 走 prune 端点，CLI 走「列出 + 逐个删」。
    pub async fn prune_images(&self) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.prune_images().await,
            EngineBackend::Cli(cli) => cli.prune_images().await,
        }
    }

    /// 清理未使用数据卷；REST 走 prune 端点，CLI 走「列出 dangling + 逐个删」。
    pub async fn prune_volumes(
        &self,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.prune_volumes().await,
            EngineBackend::Cli(cli) => cli.prune_volumes().await,
        }
    }

    pub async fn list_networks(&self) -> Result<Vec<ContainerNetworkSummary>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.list_networks().await,
            EngineBackend::Cli(cli) => cli.list_networks().await,
        }
    }

    pub async fn inspect_network(
        &self,
        network: &str,
    ) -> Result<ContainerNetworkDetails, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.inspect_network(network).await,
            EngineBackend::Cli(cli) => cli.inspect_network(network).await,
        }
    }

    pub async fn create_network(
        &self,
        spec: &transom_core::ContainerNetworkSpec,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.create_network(spec).await,
            EngineBackend::Cli(cli) => cli.create_network(spec).await,
        }
    }

    pub async fn remove_network(&self, network: &str) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.remove_network(network).await,
            EngineBackend::Cli(cli) => cli.remove_network(network).await,
        }
    }

    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.connect_network(network, container).await,
            EngineBackend::Cli(cli) => cli.connect_network(network, container).await,
        }
    }

    pub async fn disconnect_network(
        &self,
        network: &str,
        container: &str,
        force: bool,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.disconnect_network(network, container, force).await,
            EngineBackend::Cli(cli) => cli.disconnect_network(network, container, force).await,
        }
    }

    /// 清理未使用的容器网络。
    pub async fn prune_networks(
        &self,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.prune_networks().await,
            EngineBackend::Cli(cli) => cli.prune_networks().await,
        }
    }

    pub async fn container_stats(
        &self,
        container: &str,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<ContainerStats, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.container_stats(container, cancel).await,
            EngineBackend::Cli(cli) => cli.container_stats(container).await,
        }
    }

    /// 持续输出容器统计。HTTP 后端使用单条 Docker 流，CLI 后端固定节拍采样。
    ///
    /// # Errors
    /// 流建立、读取、解析或 CLI 采样失败时返回错误。
    pub async fn stream_container_stats(
        &self,
        container: &str,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        sink: &mut crate::stats::StatsSink<'_>,
    ) -> Result<(), ContainerError> {
        crate::stats::stream_container_stats(self, container, cancel, sink).await
    }

    pub async fn list_processes(
        &self,
        container: &str,
    ) -> Result<Vec<ContainerProcess>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.list_processes(container).await,
            EngineBackend::Cli(cli) => cli.list_processes(container).await,
        }
    }

    /// Compose 项目列表：两个后端都从容器列表的标签聚合。
    pub async fn compose_projects(&self) -> Result<Vec<ComposeProject>, ContainerError> {
        let containers = self.list_containers().await?;
        Ok(compose_projects(&containers))
    }

    /// 容器启停删；CLI 后端同样可用。
    pub async fn lifecycle(
        &self,
        container: &str,
        action: ContainerLifecycleAction,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.lifecycle(container, action).await,
            EngineBackend::Cli(cli) => cli.lifecycle(container, action).await,
        }
    }

    /// 动态更新容器配置。
    pub async fn update_container(
        &self,
        container: &str,
        spec: &ContainerUpdateSpec,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.update_container(container, spec).await,
            EngineBackend::Cli(cli) => cli.update_container(container, spec).await,
        }
    }

    /// 将容器就地提交为新镜像（Commit）。
    pub async fn commit_container(
        &self,
        container: &str,
        repo: &str,
        tag: Option<&str>,
        comment: Option<&str>,
        author: Option<&str>,
    ) -> Result<String, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => {
                api.commit_container(container, repo, tag, comment, author)
                    .await
            }
            EngineBackend::Cli(cli) => {
                cli.commit_container(container, repo, tag, comment, author)
                    .await
            }
        }
    }

    /// 创建并启动容器；返回新容器 ID。
    ///
    /// 设计原理（Why）：只有 REST 后端能给出结构化的创建请求（端口、挂载、重启
    /// 策略），CLI 兜底用 `docker run` 拼参数容易引入引号与转义问题，且我们的 CLI
    /// 形态没有回读新容器 ID 的稳定输出，因此明确返回不支持，让界面给出可操作的
    /// 提示而不是静默失败。
    ///
    /// # Errors
    /// CLI 后端返回 [`ContainerError::Unsupported`]；引擎拒绝时透传
    /// [`ContainerError::Api`]。
    pub async fn create_container(
        &self,
        spec: &transom_core::ContainerCreateSpec,
    ) -> Result<String, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.create_container(spec).await,
            EngineBackend::Cli(_) => Err(ContainerError::Unsupported {
                operation: "创建容器（需要引擎 REST 接口）".to_string(),
            }),
        }
    }

    /// 在远端镜像仓库中检索镜像。
    pub async fn search_images(
        &self,
        term: &str,
        limit: Option<u32>,
    ) -> Result<Vec<transom_core::ImageSearchResult>, ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.search_images(term, limit).await,
            EngineBackend::Cli(_) => Err(ContainerError::Unsupported {
                operation: "检索远端镜像（需要引擎 REST 接口）".to_string(),
            }),
        }
    }

    /// 拉取镜像；HTTP 后端上报进度，CLI 后端只在结束时返回。
    pub async fn pull_image(
        &self,
        reference: &str,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.pull_image(reference, progress).await,
            EngineBackend::Cli(cli) => cli.pull_image(reference).await,
        }
    }

    /// 带任务追踪与分层进度的镜像拉取。
    pub async fn pull_image_task(
        &self,
        reference: &str,
        auth: Option<&ContainerRegistryAuth>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => {
                api.pull_image_task(reference, auth, cancel, progress).await
            }
            EngineBackend::Cli(cli) => cli.pull_image(reference).await,
        }
    }

    /// 推送镜像；HTTP 后端流式上报进度与处理鉴权，CLI 后端执行 docker push。
    pub async fn push_image(
        &self,
        image: &str,
        tag: Option<&str>,
        auth: Option<&ContainerRegistryAuth>,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.push_image(image, tag, auth, progress).await,
            EngineBackend::Cli(cli) => {
                let ref_str = match tag {
                    Some(t) if !t.is_empty() => format!("{image}:{t}"),
                    _ => image.to_string(),
                };
                cli.push_image(&ref_str).await
            }
        }
    }

    /// 构建镜像；HTTP 与 CLI 后端均支持流式进度上报与取消。
    pub async fn build_image(
        &self,
        spec: &ContainerBuildSpec,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        progress: &mut (dyn FnMut(String) + Send),
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.build_image(spec, cancel, progress).await,
            EngineBackend::Cli(cli) => cli.build_image(spec, cancel, progress).await,
        }
    }

    pub async fn remove_image(&self, image: &str, force: bool) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.remove_image(image, force).await,
            EngineBackend::Cli(cli) => cli.remove_image(image, force).await,
        }
    }

    /// HTTP 客户端入口：日志、exec 与文件模块需要它。
    /// 重命名容器；CLI 兜底后端同样可用。
    ///
    /// # Errors
    /// 名称非法或引擎拒绝时返回 [`ContainerError`]。
    pub async fn rename_container(
        &self,
        container: &str,
        name: &str,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.rename_container(container, name).await,
            EngineBackend::Cli(cli) => cli.rename_container(container, name).await,
        }
    }

    pub fn http(&self) -> Result<&DockerApi, ContainerError> {
        self.api()
    }

    /// CLI 入口：exec 与日志跟随需要它。
    pub fn cli_backend_ref(&self) -> Option<&CliBackend> {
        self.cli()
    }

    /// 监听容器引擎全局长连接事件流。
    pub async fn listen_events(
        &self,
        event_tx: tokio::sync::mpsc::Sender<transom_core::EngineEvent>,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ContainerError> {
        match &self.backend {
            EngineBackend::Http(api) => api.listen_events(event_tx, shutdown_rx).await,
            EngineBackend::Cli(cli) => cli.listen_events(event_tx, shutdown_rx).await,
        }
    }
}

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
    use super::{Engine, EngineBackend, transport_for};
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
