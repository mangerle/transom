//! 引擎操作面：按资源域组织的统一分派方法。

use super::{Engine, EngineBackend};
use crate::error::ContainerError;
use transom_core::{
    ComposeProject, ContainerBuildSpec, ContainerDetails, ContainerImageDetails,
    ContainerImageLayer, ContainerImageSummary, ContainerLifecycleAction, ContainerNetworkDetails,
    ContainerNetworkSummary, ContainerProcess, ContainerRegistryAuth, ContainerStats,
    ContainerSummary, ContainerUpdateSpec, ContainerVolumeDetails, ContainerVolumeSummary,
    compose_projects,
};

impl Engine {
    pub async fn list_containers(&self) -> Result<Vec<ContainerSummary>, ContainerError> {
        dispatch!(self, list_containers())
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
        dispatch!(self, list_images())
    }

    pub async fn inspect_container(
        &self,
        container: &str,
    ) -> Result<ContainerDetails, ContainerError> {
        dispatch!(self, inspect_container(container))
    }

    pub async fn container_changes(
        &self,
        container: &str,
    ) -> Result<Vec<transom_core::ContainerFileSystemChange>, ContainerError> {
        dispatch!(self, container_changes(container))
    }

    pub async fn system_df(
        &self,
    ) -> Result<transom_core::ContainerSystemDiskUsage, ContainerError> {
        dispatch!(self, system_df())
    }

    pub async fn system_prune(
        &self,
        all: bool,
        volumes: bool,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        dispatch!(self, system_prune(all, volumes))
    }

    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<ContainerImageDetails, ContainerError> {
        dispatch!(self, inspect_image(image))
    }

    pub async fn image_history(
        &self,
        image: &str,
    ) -> Result<Vec<ContainerImageLayer>, ContainerError> {
        dispatch!(self, image_history(image))
    }

    pub async fn tag_image(
        &self,
        image: &str,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<(), ContainerError> {
        dispatch!(self, tag_image(image, repo, tag))
    }

    pub async fn list_volumes(&self) -> Result<Vec<ContainerVolumeSummary>, ContainerError> {
        dispatch!(self, list_volumes())
    }

    pub async fn inspect_volume(
        &self,
        name: &str,
    ) -> Result<ContainerVolumeDetails, ContainerError> {
        dispatch!(self, inspect_volume(name))
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        dispatch!(self, create_volume(name, driver))
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<(), ContainerError> {
        dispatch!(self, remove_volume(name, force))
    }

    /// 清理悬空镜像；与卷一样，REST 走 prune 端点，CLI 走「列出 + 逐个删」。
    pub async fn prune_images(&self) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        dispatch!(self, prune_images())
    }

    /// 清理未使用数据卷；REST 走 prune 端点，CLI 走「列出 dangling + 逐个删」。
    pub async fn prune_volumes(
        &self,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        dispatch!(self, prune_volumes())
    }

    pub async fn list_networks(&self) -> Result<Vec<ContainerNetworkSummary>, ContainerError> {
        dispatch!(self, list_networks())
    }

    pub async fn inspect_network(
        &self,
        network: &str,
    ) -> Result<ContainerNetworkDetails, ContainerError> {
        dispatch!(self, inspect_network(network))
    }

    pub async fn create_network(
        &self,
        spec: &transom_core::ContainerNetworkSpec,
    ) -> Result<(), ContainerError> {
        dispatch!(self, create_network(spec))
    }

    pub async fn remove_network(&self, network: &str) -> Result<(), ContainerError> {
        dispatch!(self, remove_network(network))
    }

    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
    ) -> Result<(), ContainerError> {
        dispatch!(self, connect_network(network, container))
    }

    pub async fn disconnect_network(
        &self,
        network: &str,
        container: &str,
        force: bool,
    ) -> Result<(), ContainerError> {
        dispatch!(self, disconnect_network(network, container, force))
    }

    /// 清理未使用的容器网络。
    pub async fn prune_networks(
        &self,
    ) -> Result<transom_core::ContainerPruneReport, ContainerError> {
        dispatch!(self, prune_networks())
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
        dispatch!(self, list_processes(container))
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
        dispatch!(self, lifecycle(container, action))
    }

    /// 动态更新容器配置。
    pub async fn update_container(
        &self,
        container: &str,
        spec: &ContainerUpdateSpec,
    ) -> Result<(), ContainerError> {
        dispatch!(self, update_container(container, spec))
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
        dispatch!(self, build_image(spec, cancel, progress))
    }

    pub async fn remove_image(&self, image: &str, force: bool) -> Result<(), ContainerError> {
        dispatch!(self, remove_image(image, force))
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
        dispatch!(self, rename_container(container, name))
    }

    /// 监听容器引擎全局长连接事件流。
    pub async fn listen_events(
        &self,
        event_tx: tokio::sync::mpsc::Sender<transom_core::EngineEvent>,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ContainerError> {
        dispatch!(self, listen_events(event_tx, shutdown_rx))
    }
}
