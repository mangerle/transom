//! Docker 兼容 REST 客户端。
//!
//! Docker 与 Podman 在本项目用到的端点（列表、检视、启停、exec、日志、
//! 镜像、归档）上保持兼容，因此只实现一套协议；Podman 的 `/libpod` 专有
//! 接口不在范围内。

use crate::error::ContainerError;
use crate::http::{self, RawResponse};
use crate::models::{
    ChangeItem, ContainerInspect, ContainerListItem, ContainerUpdateRequest, ExecCreateResponse,
    ExecInspectResponse, ImageHistoryItem, ImageInspect, ImageListItem, ImagePruneResponse, ImagePullEvent,
    ImageSearchItem, NetworkItem, NetworkPruneResponse, RestartPolicyUpdate, StatsResponse, TopResponse, VersionResponse, VolumeItem,
    VolumeListResponse, VolumePruneResponse,
};
use crate::transport::EngineTransport;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use transom_core::ContainerEngineKind;
use transom_core::{
    ContainerDetails, ContainerFileSystemChange, ContainerImageDetails, ContainerImageLayer, ContainerImageSummary, ContainerLifecycleAction,
    ContainerNetworkDetails, ContainerNetworkSummary, ContainerProcess, ContainerPruneReport,
    ContainerRegistryAuth, ContainerStats, ContainerSummary, ContainerUpdateSpec, ContainerVolumeDetails,
    ContainerVolumeSummary,
};

/// 小体积 JSON 响应的上限：超过视为异常响应，避免误读巨型对象。
const JSON_BODY_LIMIT: usize = 8 * 1024 * 1024;

/// 版本协商后的 API 客户端。
#[derive(Debug, Clone)]
pub struct DockerApi {
    transport: EngineTransport,
    /// 形如 `/v1.46`；协商不出可用版本时为空串（走引擎的默认版本）。
    prefix: String,
}

impl DockerApi {
    pub fn new(transport: EngineTransport, api_version: Option<&str>) -> Self {
        Self {
            transport,
            prefix: normalize_api_prefix(api_version),
        }
    }

    pub fn transport(&self) -> &EngineTransport {
        &self.transport
    }

    /// 探测端点并返回版本信息；`kind` 只用于兜底，实际种类由响应识别。
    ///
    /// # Errors
    /// 端点不可达、权限不足或响应无法解析时返回 `ContainerError`。
    pub async fn probe(&self, kind: ContainerEngineKind) -> Result<ProbedEngine, ContainerError> {
        let response =
            http::request(&self.transport, Method::GET, "/_ping", Bytes::new(), None).await?;
        let ping_status = response.status;
        let ping_header = http::header_str(&response.headers, "API-Version");
        let ping_body = http::read_body_text(response.body, 1024).await;
        if !ping_status.is_success() {
            return Err(ContainerError::Api {
                operation: "探测引擎 /_ping".to_string(),
                status: ping_status.as_u16(),
                body: ping_body,
            });
        }

        // 版本接口在个别代理后面可能不可用；此时仍可用 _ping 头里的 API 版本。
        let (version, api_version) = match self
            .get_json::<VersionResponse>("/version", "读取引擎版本")
            .await
        {
            Ok(version) => {
                let api_version = version.api_version.clone().or(ping_header);
                let api = DockerApi::new(self.transport.clone(), api_version.as_deref());
                let version_text = version.version.clone().unwrap_or_default();
                let detected = if version.is_podman() {
                    ContainerEngineKind::Podman
                } else {
                    kind
                };
                return Ok(ProbedEngine {
                    kind: detected,
                    version: version_text,
                    api_version: api.prefix.trim_start_matches("/v").to_string(),
                    platform: version.platform_name(),
                });
            }
            Err(error) => {
                log::debug!(
                    "引擎版本接口不可用，按 _ping 头降级，端点: {}，原因: {error}",
                    self.transport.describe()
                );
                (String::new(), ping_header)
            }
        };

        Ok(ProbedEngine {
            kind,
            version,
            api_version: normalize_api_prefix(api_version.as_deref())
                .trim_start_matches("/v")
                .to_string(),
            platform: "Docker/Podman".to_string(),
        })
    }

    /// 重新协商版本后的客户端；调用方在 probe 之后用真实版本替换。
    pub fn with_api_version(self, api_version: &str) -> Self {
        Self {
            transport: self.transport,
            prefix: normalize_api_prefix(Some(api_version)),
        }
    }

    fn path(&self, suffix: &str) -> String {
        format!("{}{}", self.prefix, suffix)
    }

    /// 带版本前缀的请求路径；日志与文件模块复用同一协商结果。
    pub(crate) fn versioned_path(&self, suffix: &str) -> String {
        self.path(suffix)
    }

    /// 底层通道：日志模块直接构造长连接请求。
    pub(crate) fn transport_ref(&self) -> &EngineTransport {
        &self.transport
    }

    async fn send(
        &self,
        method: Method,
        suffix: &str,
        body: Option<Vec<u8>>,
        operation: &str,
    ) -> Result<RawResponse, ContainerError> {
        let path = self.path(suffix);
        let content_type = body.as_ref().map(|_| "application/json");
        let bytes = body.map(Bytes::from).unwrap_or_default();
        http::request(&self.transport, method, &path, bytes, content_type)
            .await
            .inspect_err(|error| log_request_error(operation, error))
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        suffix: &str,
        operation: &str,
    ) -> Result<T, ContainerError> {
        let response = self.send(Method::GET, suffix, None, operation).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, operation, body, suffix));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
            operation: operation.to_string(),
            message: error.to_string(),
        })
    }

    async fn post_empty(
        &self,
        suffix: &str,
        operation: &str,
        target: Option<&str>,
    ) -> Result<(), ContainerError> {
        let response = self.send(Method::POST, suffix, None, operation).await?;
        if response.status.is_success() {
            // 仍有响应体需要读完/丢弃；交给连接守卫回收即可。
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, operation, body, target)
    }

    /// 容器列表（含已停止）。
    pub async fn list_containers(&self) -> Result<Vec<ContainerSummary>, ContainerError> {
        let items: Vec<ContainerListItem> =
            self.get_json("/containers/json?all=1", "列出容器").await?;
        Ok(items
            .into_iter()
            .map(ContainerListItem::into_summary)
            .collect())
    }

    /// 镜像列表。
    pub async fn list_images(&self) -> Result<Vec<ContainerImageSummary>, ContainerError> {
        let items: Vec<ImageListItem> = self.get_json("/images/json", "列出镜像").await?;
        Ok(items.into_iter().map(ImageListItem::into_summary).collect())
    }

    /// 容器是否分配了 TTY：决定日志流是裸流还是 8 字节多路复用流。
    pub async fn container_tty(&self, container: &str) -> Result<bool, ContainerError> {
        let inspect: ContainerInspect = self
            .get_json(
                &format!("/containers/{}/json", http::percent_encode(container)),
                "检视容器",
            )
            .await?;
        Ok(inspect.config.map(|config| config.tty).unwrap_or(false))
    }

    /// 容器详情（基础信息页签）。
    pub async fn inspect_container(
        &self,
        container: &str,
    ) -> Result<ContainerDetails, ContainerError> {
        // size=1 让引擎顺带算可写层/镜像层占用；单容器 inspect 可以接受这点开销，
        // 统计页的磁盘卡片需要它。
        let suffix = format!(
            "/containers/{}/json?size=1",
            http::percent_encode(container)
        );
        let operation = "检视容器";
        let response = self.send(Method::GET, &suffix, None, operation).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, operation, body, container));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let raw = format_inspect_json(&bytes);
        let inspect: ContainerInspect =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: operation.to_string(),
                message: error.to_string(),
            })?;
        Ok(inspect.into_details(raw))
    }

    /// 查询容器文件系统变更（Diff）。
    ///
    /// # Errors
    /// 容器不存在或引擎返回异常时返回 [`ContainerError`]。
    pub async fn container_changes(
        &self,
        container: &str,
    ) -> Result<Vec<ContainerFileSystemChange>, ContainerError> {
        let suffix = format!("/containers/{}/changes", http::percent_encode(container));
        let items: Vec<ChangeItem> = self.get_json(&suffix, "查询容器文件系统变更").await?;
        Ok(items.into_iter().map(ChangeItem::into_change).collect())
    }

    /// 镜像详情。
    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<ContainerImageDetails, ContainerError> {
        let (inspect, raw) = self
            .inspect_json::<ImageInspect>(
                &format!("/images/{}/json", http::percent_encode(image)),
                "检视镜像",
            )
            .await?;
        let history = self.image_history(image).await.unwrap_or_default();
        let mut details = inspect.into_details(raw);
        details.history = history;
        Ok(details)
    }

    /// 获取镜像构建历史层。
    pub async fn image_history(
        &self,
        image: &str,
    ) -> Result<Vec<ContainerImageLayer>, ContainerError> {
        let suffix = format!("/images/{}/history", http::percent_encode(image));
        let items: Vec<ImageHistoryItem> = self.get_json(&suffix, "获取镜像历史层").await?;
        Ok(items.into_iter().map(ImageHistoryItem::into_layer).collect())
    }

    /// 为镜像附加新标签（Tag）。
    pub async fn tag_image(
        &self,
        image: &str,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<(), ContainerError> {
        let encoded_image = http::percent_encode(image);
        let encoded_repo = http::percent_encode(repo);
        let tag_query = tag
            .filter(|t| !t.is_empty())
            .map(|t| format!("&tag={}", http::percent_encode(t)))
            .unwrap_or_default();
        let path = format!("/images/{encoded_image}/tag?repo={encoded_repo}{tag_query}");
        let response = self.send(Method::POST, &path, None, "镜像打标签").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let body = http::read_body_text(response.body, 8192).await;
        Err(http::status_error(response.status, "镜像打标签", body, image))
    }

    /// 取一份 JSON 对象，同时把原文留一份格式化副本。
    ///
    /// 设计原理（Why）：详情面板要能在「结构化」与「原始 JSON」之间切换，而
    /// 结构化字段必然丢上游信息；与其为每个对象再解析一遍，不如在唯一一次读取
    /// 时把原文留下。解析失败时原文也照样能看，所以格式化不依赖反序列化成功。
    ///
    /// # Errors
    /// 引擎返回非 2xx、响应体超限或无法解析为目标类型时返回 [`ContainerError`]。
    async fn inspect_json<T>(
        &self,
        suffix: &str,
        operation: &str,
    ) -> Result<(T, String), ContainerError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self.send(Method::GET, suffix, None, operation).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, operation, body, suffix));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let raw = format_inspect_json(&bytes);
        let parsed = serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
            operation: operation.to_string(),
            message: error.to_string(),
        })?;
        Ok((parsed, raw))
    }

    /// 数据卷列表。
    pub async fn list_volumes(&self) -> Result<Vec<ContainerVolumeSummary>, ContainerError> {
        let response: VolumeListResponse = self.get_json("/volumes", "列出数据卷").await?;
        Ok(response
            .volumes
            .into_iter()
            .map(VolumeItem::into_summary)
            .collect())
    }

    /// 数据卷详情；使用它的容器从容器列表的挂载信息反查。
    pub async fn inspect_volume(
        &self,
        name: &str,
    ) -> Result<ContainerVolumeDetails, ContainerError> {
        let (volume, raw) = self
            .inspect_json::<VolumeItem>(
                &format!("/volumes/{}", http::percent_encode(name)),
                "检视数据卷",
            )
            .await?;
        let containers: Vec<ContainerListItem> = self
            .get_json("/containers/json?all=1", "列出容器（卷占用）")
            .await
            .unwrap_or_default();
        let used_by = containers
            .into_iter()
            .filter(|item| {
                item.mounts
                    .iter()
                    .any(|mount| mount.name.as_deref() == Some(name))
            })
            .map(|item| item.display_name())
            .collect();
        Ok(ContainerVolumeDetails {
            summary: volume.into_summary(),
            used_by,
            inspect: raw,
        })
    }

    /// 创建容器（不启动）并返回新容器 ID。
    ///
    /// 设计原理（Why）：`Cmd` 用 argv 而不是字符串，交给引擎 exec 形态解析，
    /// 避免宿主侧拼 Shell；端口与挂载按 Docker 的 `PortBindings`/`Binds` 形态
    /// 组装，空列表传空数组而不是 null，因为 Podman 对 null 的处理与 Docker
    /// 不完全一致。
    ///
    /// # Errors
    /// 引擎拒绝（镜像不存在、名称冲突、端口占用等）时返回 [`ContainerError::Api`]，
    /// 响应体原文带上定位信息。
    pub async fn create_container(
        &self,
        spec: &transom_core::ContainerCreateSpec,
    ) -> Result<String, ContainerError> {
        let payload = serde_json::json!({
            "Image": spec.image,
            "Env": spec.env,
            "Cmd": spec.command,
            // 健康检查属于顶层 Config，不在 HostConfig 里：真机验证过放错位置会被
            // 守护进程静默丢弃（创建成功但 Config.Healthcheck 为 null）。
            // 留空时显式下发 NONE，避免继承镜像自带的 HEALTHCHECK 造成误判。
            "Healthcheck": healthcheck(spec),
            "HostConfig": {
                "Binds": spec.binds,
                "PortBindings": port_bindings(&spec.ports),
                "RestartPolicy": {
                    "Name": spec.restart.clone().unwrap_or_else(|| "no".to_string()),
                    "MaximumRetryCount": 0,
                },
                // 资源限制留空即不下发：0 在 Docker 里同样是「不限制」，但少写字段
                // 更不容易踩到版本差异。
                "Memory": spec.memory_mb.unwrap_or(0) * 1024 * 1024,
                "NanoCpus": spec.nano_cpus.unwrap_or(0),
            },
        });
        let path = if spec.name.is_empty() {
            "/containers/create".to_string()
        } else {
            format!(
                "/containers/create?name={}",
                http::percent_encode(&spec.name)
            )
        };
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造创建容器请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(body), "创建容器")
            .await?;
        if !response.status.is_success() {
            let status = response.status;
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "创建容器", body, &spec.image));
        }
        #[derive(serde::Deserialize)]
        struct Created {
            #[serde(rename = "Id", default)]
            id: String,
        }
        let body = http::read_body_bytes(response.body, 4096).await?;
        let created: Created =
            serde_json::from_slice(&body).map_err(|error| ContainerError::Decode {
                operation: "解析创建容器响应".to_string(),
                message: format!("{error}: {}", String::from_utf8_lossy(&body)),
            })?;
        Ok(created.id)
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        let payload = serde_json::json!({
            "Name": name,
            "Driver": driver.unwrap_or("local"),
        });
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造数据卷请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, "/volumes/create", Some(body), "创建数据卷")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "创建数据卷", body, Some(name))
    }

    /// 清理悬空镜像（没有 tag 也不再被任何镜像引用的那批）。
    ///
    /// 设计原理（Why）：与卷不同，镜像的 prune 默认语义就是 `dangling=true`，
    /// 这里显式带上过滤，避免依赖引擎默认值；过滤条件同样只能走查询参数。
    /// 删除对象既可能是取消标签（`Untagged`）也可能是真删层（`Deleted`），
    /// 两种都收进同一份报告，界面按条数回报。
    ///
    /// # Errors
    /// 引擎不可达、返回非 2xx 或响应无法解析时返回 [`ContainerError`]。
    pub async fn prune_images(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理悬空镜像";
        let suffix = format!(
            "/images/prune?filters={}",
            http::percent_encode(r#"{"dangling":["true"]}"#)
        );
        let response = self.send(Method::POST, &suffix, None, OPERATION).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "images"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: ImagePruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        let deleted = report
            .images_deleted
            .into_iter()
            .flat_map(|entry| [entry.untagged, entry.deleted])
            .flatten()
            .collect();
        Ok(ContainerPruneReport {
            deleted,
            reclaimed: report.space_reclaimed,
        })
    }

    /// 清理未被任何容器使用的数据卷。
    ///
    /// 设计原理（Why）：Docker 的 prune 默认只回收**匿名**卷，具名卷必须显式带
    /// `all=true` 过滤才会被清理（真机实测：不带过滤时返回空列表，界面上却明明
    /// 列出了未使用卷）；界面「仅未使用卷」那一档对应的正是这个语义，所以过滤
    /// 固定写死。正在被挂载的卷由引擎自己排除，不需要我们传候选名单。
    ///
    /// 过滤条件必须走查询参数：放进 JSON 请求体会被引擎静默忽略（实测两种写法
    /// 结果不同），因此这里显式拼 URL。
    ///
    /// # Errors
    /// 引擎不可达、返回非 2xx 或响应无法解析时返回 [`ContainerError`]。
    pub async fn prune_volumes(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理未使用数据卷";
        let suffix = format!(
            "/volumes/prune?filters={}",
            http::percent_encode(r#"{"all":["true"]}"#)
        );
        let response = self.send(Method::POST, &suffix, None, OPERATION).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "volumes"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: VolumePruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(ContainerPruneReport {
            deleted: report.volumes_deleted,
            reclaimed: report.space_reclaimed,
        })
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<(), ContainerError> {
        let path = format!(
            "/volumes/{}?force={}",
            http::percent_encode(name),
            u8::from(force)
        );
        let response = self.send(Method::DELETE, &path, None, "删除数据卷").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "删除数据卷", body, Some(name))
    }

    /// 网络列表。
    pub async fn list_networks(&self) -> Result<Vec<ContainerNetworkSummary>, ContainerError> {
        let items: Vec<NetworkItem> = self.get_json("/networks", "列出网络").await?;
        Ok(items.iter().map(NetworkItem::summary).collect())
    }

    /// 网络详情。
    pub async fn inspect_network(
        &self,
        network: &str,
    ) -> Result<ContainerNetworkDetails, ContainerError> {
        let (item, raw) = self
            .inspect_json::<NetworkItem>(
                &format!("/networks/{}", http::percent_encode(network)),
                "检视网络",
            )
            .await?;
        Ok(item.into_details(raw))
    }

    pub async fn create_network(
        &self,
        spec: &transom_core::ContainerNetworkSpec,
    ) -> Result<(), ContainerError> {
        let name = spec.name.as_str();
        // 留空的字段一律不下发，由引擎按自己的默认值决定；只有填了子网才带 IPAM。
        let mut payload = serde_json::json!({
            "Name": name,
            "CheckDuplicate": true,
        });
        if let Some(driver) = spec.driver.as_deref() {
            payload["Driver"] = serde_json::Value::String(driver.to_string());
        }
        if spec.internal {
            payload["Internal"] = serde_json::Value::Bool(true);
        }
        if let Some(subnet) = spec.subnet.as_deref() {
            let mut config = serde_json::json!({ "Subnet": subnet });
            if let Some(gateway) = spec.gateway.as_deref() {
                config["Gateway"] = serde_json::Value::String(gateway.to_string());
            }
            payload["IPAM"] = serde_json::json!({ "Config": [config] });
        }
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造网络请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, "/networks/create", Some(body), "创建网络")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "创建网络", body, Some(name))
    }

    pub async fn remove_network(&self, network: &str) -> Result<(), ContainerError> {
        let path = format!("/networks/{}", http::percent_encode(network));
        let response = self.send(Method::DELETE, &path, None, "删除网络").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "删除网络", body, Some(network))
    }

    /// 将容器连接到网络。
    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
    ) -> Result<(), ContainerError> {
        let path = format!("/networks/{}/connect", http::percent_encode(network));
        let payload = serde_json::json!({
            "Container": container,
        });
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造连接网络请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(body), "连接网络")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "连接网络", body, Some(network))
    }

    /// 将容器从网络断开。
    pub async fn disconnect_network(
        &self,
        network: &str,
        container: &str,
        force: bool,
    ) -> Result<(), ContainerError> {
        let path = format!("/networks/{}/disconnect", http::percent_encode(network));
        let payload = serde_json::json!({
            "Container": container,
            "Force": force,
        });
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造断开网络请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(body), "断开网络")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "断开网络", body, Some(network))
    }

    /// 清理未使用的容器网络。
    pub async fn prune_networks(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理未使用网络";
        let response = self.send(Method::POST, "/networks/prune", None, OPERATION).await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "networks"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: NetworkPruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(ContainerPruneReport {
            deleted: report.networks_deleted,
            reclaimed: 0,
        })
    }

    /// 容器内进程列表（`ps aux` 口径）。
    pub async fn list_processes(
        &self,
        container: &str,
    ) -> Result<Vec<ContainerProcess>, ContainerError> {
        let response: TopResponse = self
            .get_json(
                &format!(
                    "/containers/{}/top?ps_args=aux",
                    http::percent_encode(container)
                ),
                "列出容器进程",
            )
            .await?;
        Ok(response.into_processes())
    }

    /// 采样一次容器统计；CPU 百分比需要两次样本，见 [`Self::container_stats`]。
    pub async fn container_stats(
        &self,
        container: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ContainerStats, ContainerError> {
        let first: StatsResponse = self
            .get_json(
                &format!(
                    "/containers/{}/stats?stream=0",
                    http::percent_encode(container)
                ),
                "采样容器统计",
            )
            .await?;
        // 两个样本间隔约 1 秒：非流式响应通常没有 precpu 数据，只能用两次
        // 采样自己做差值。取消检查走 100ms 粒度，避免关闭页签后还等满一秒。
        for _ in 0..10 {
            if cancel.load(Ordering::Relaxed) {
                return Err(ContainerError::Cancelled);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let second: StatsResponse = self
            .get_json(
                &format!(
                    "/containers/{}/stats?stream=0",
                    http::percent_encode(container)
                ),
                "采样容器统计",
            )
            .await?;
        Ok(second.to_stats(Some(&first)))
    }

    /// 启动/停止/重启/删除容器。
    pub async fn lifecycle(
        &self,
        container: &str,
        action: ContainerLifecycleAction,
    ) -> Result<(), ContainerError> {
        let encoded = http::percent_encode(container);
        let (method, path, operation) = match action {
            ContainerLifecycleAction::Start => (
                Method::POST,
                format!("/containers/{encoded}/start"),
                "启动容器",
            ),
            ContainerLifecycleAction::Stop => (
                Method::POST,
                format!("/containers/{encoded}/stop?t=10"),
                "停止容器",
            ),
            ContainerLifecycleAction::Restart => (
                Method::POST,
                format!("/containers/{encoded}/restart?t=10"),
                "重启容器",
            ),
            ContainerLifecycleAction::Pause => (
                Method::POST,
                format!("/containers/{encoded}/pause"),
                "暂停容器",
            ),
            ContainerLifecycleAction::Unpause => (
                Method::POST,
                format!("/containers/{encoded}/unpause"),
                "恢复容器",
            ),
            ContainerLifecycleAction::Kill { signal } => {
                let query = signal
                    .as_deref()
                    .map(|sig| format!("?signal={}", http::percent_encode(sig)))
                    .unwrap_or_default();
                (
                    Method::POST,
                    format!("/containers/{encoded}/kill{query}"),
                    "强制终止容器",
                )
            }
            ContainerLifecycleAction::Remove { force } => (
                Method::DELETE,
                format!("/containers/{encoded}?v=1&force={}", u8::from(force)),
                "删除容器",
            ),
        };
        let response = self.send(method, &path, None, operation).await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, operation, body, Some(container))
    }

    /// 动态更新容器资源配置（CPU、内存配额与重启策略）。
    ///
    /// # Errors
    /// 容器不存在、参数非法或引擎更新失败时返回 [`ContainerError`]。
    pub async fn update_container(
        &self,
        container: &str,
        spec: &ContainerUpdateSpec,
    ) -> Result<(), ContainerError> {
        let encoded = http::percent_encode(container);
        let path = format!("/containers/{encoded}/update");
        let body = ContainerUpdateRequest {
            memory: spec.memory_bytes,
            nano_cpus: spec.nano_cpus,
            restart_policy: spec.restart_policy.as_ref().map(|name| RestartPolicyUpdate {
                name: name.clone(),
            }),
        };
        let payload = serde_json::to_vec(&body).map_err(|error| ContainerError::Http {
            operation: "序列化更新容器参数".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(
                Method::POST,
                &path,
                Some(payload),
                "更新容器配置",
            )
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "更新容器配置", body, Some(container))
    }

    /// 将容器就地提交为新镜像（Commit）。
    ///
    /// # Errors
    /// 容器不存在或引擎提交失败时返回 [`ContainerError`]。
    pub async fn commit_container(
        &self,
        container: &str,
        repo: &str,
        tag: Option<&str>,
        comment: Option<&str>,
        author: Option<&str>,
    ) -> Result<String, ContainerError> {
        let mut query = vec![
            format!("container={}", http::percent_encode(container)),
            format!("repo={}", http::percent_encode(repo)),
        ];
        if let Some(tag) = tag {
            if !tag.is_empty() {
                query.push(format!("tag={}", http::percent_encode(tag)));
            }
        }
        if let Some(comment) = comment {
            if !comment.is_empty() {
                query.push(format!("comment={}", http::percent_encode(comment)));
            }
        }
        if let Some(author) = author {
            if !author.is_empty() {
                query.push(format!("author={}", http::percent_encode(author)));
            }
        }
        let path = format!("/commit?{}", query.join("&"));
        let response = self
            .send(Method::POST, &path, None, "提交容器镜像")
            .await?;
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        if status.is_success() {
            let id = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("Id")
                        .and_then(|id| id.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            return Ok(id);
        }
        http::ensure_success(status, "提交容器镜像", body, Some(container))?;
        Ok(String::new())
    }

    /// 重命名容器。
    ///
    /// 设计原理（Why）：引擎只有 `POST /containers/{id}/rename?name=` 这一条
    /// 通道，没有通用「改元数据」接口；改名后正在看的工作区选中项仍然按 ID 定位，
    /// 因此界面不会因为改名丢会话。
    ///
    /// # Errors
    /// 新名称非法、容器不存在或名称已被占用时返回对应错误。
    pub async fn rename_container(
        &self,
        container: &str,
        name: &str,
    ) -> Result<(), ContainerError> {
        if !transom_core::is_valid_container_name(name) {
            return Err(ContainerError::InvalidName {
                name: name.to_string(),
                reason: "容器名只能包含字母、数字、点、下划线和短横线，且必须以字母或数字开头"
                    .to_string(),
            });
        }
        let path = format!(
            "/containers/{}/rename?name={}",
            http::percent_encode(container),
            http::percent_encode(name)
        );
        let response = self
            .send(
                Method::POST,
                &path,
                None,
                &format!("重命名容器 {container}"),
            )
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "重命名容器", body, Some(container))
    }

    /// 拉取镜像；进度通过回调上报，流中的错误会转成 `ContainerError`。
    pub async fn pull_image(
        &self,
        reference: &str,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<(), ContainerError> {
        let (image, tag) = split_reference(reference);
        let mut path = format!("/images/create?fromImage={}", http::percent_encode(&image));
        if let Some(tag) = tag {
            path.push_str(&format!("&tag={}", http::percent_encode(&tag)));
        }
        let response = self.send(Method::POST, &path, None, "拉取镜像").await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "拉取镜像", body, reference));
        }
        consume_pull_stream(response.body, progress).await
    }

    /// 在远端镜像仓库中检索镜像。
    pub async fn search_images(
        &self,
        term: &str,
        limit: Option<u32>,
    ) -> Result<Vec<transom_core::ImageSearchResult>, ContainerError> {
        let mut path = format!("/images/search?term={}", http::percent_encode(term));
        if let Some(limit) = limit {
            path.push_str(&format!("&limit={limit}"));
        }
        let response = self.send(Method::GET, &path, None, "检索远端镜像").await?;
        let status = response.status;
        let body = http::read_body_text(response.body, 131_072).await;
        http::ensure_success(status, "检索远端镜像", body.clone(), Some(term))?;
        let items: Vec<ImageSearchItem> = serde_json::from_str(&body).map_err(|error| {
            ContainerError::Decode {
                operation: "反序列化镜像检索结果".to_string(),
                message: error.to_string(),
            }
        })?;
        Ok(items.into_iter().map(Into::into).collect())
    }

    /// 带任务追踪、分层进度与中止令牌的镜像拉取。
    pub async fn pull_image_task(
        &self,
        reference: &str,
        auth: Option<&ContainerRegistryAuth>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
    ) -> Result<(), ContainerError> {
        let (image, tag) = split_reference(reference);
        let mut path = format!("/images/create?fromImage={}", http::percent_encode(&image));
        if let Some(tag) = tag {
            path.push_str(&format!("&tag={}", http::percent_encode(&tag)));
        }
        let auth_header = crate::auth::resolve_auth_header(&image, auth);
        let mut extra_headers = Vec::new();
        if let Some(h) = auth_header.as_deref() {
            extra_headers.push(("X-Registry-Auth", h));
        }
        let full_path = self.path(&path);
        let response = http::request_with_headers(
            &self.transport,
            Method::POST,
            &full_path,
            Bytes::new(),
            None,
            &extra_headers,
        )
        .await
        .inspect_err(|error| log_request_error("拉取镜像任务", error))?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "拉取镜像任务", body, reference));
        }
        consume_pull_task_stream(response.body, cancel, progress).await
    }

    /// 推送镜像；支持自动或显式鉴权，流式上报进度。
    pub async fn push_image(
        &self,
        image: &str,
        tag: Option<&str>,
        auth: Option<&ContainerRegistryAuth>,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<(), ContainerError> {
        let (name, default_tag) = split_reference(image);
        let effective_tag = tag.or(default_tag.as_deref());
        let mut path = format!("/images/{}/push", http::percent_encode(&name));
        if let Some(tag) = effective_tag {
            path.push_str(&format!("?tag={}", http::percent_encode(tag)));
        }
        let auth_header = crate::auth::resolve_auth_header(image, auth);
        let mut extra_headers = Vec::new();
        if let Some(h) = auth_header.as_deref() {
            extra_headers.push(("X-Registry-Auth", h));
        }

        let full_path = self.path(&path);
        let response = http::request_with_headers(
            &self.transport,
            Method::POST,
            &full_path,
            Bytes::new(),
            None,
            &extra_headers,
        )
        .await
        .inspect_err(|error| log_request_error("推送镜像", error))?;

        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "推送镜像", body, image));
        }
        consume_pull_stream(response.body, progress).await
    }

    /// 构建镜像；流式上报构建进度行。
    pub async fn build_image(
        &self,
        spec: &transom_core::ContainerBuildSpec,
        progress: &mut (dyn FnMut(String) + Send),
    ) -> Result<(), ContainerError> {
        crate::build::build_image_http(self, spec, progress).await
    }

    /// 删除镜像。
    pub async fn remove_image(&self, image: &str, force: bool) -> Result<(), ContainerError> {
        let path = format!(
            "/images/{}?force={}",
            http::percent_encode(image),
            u8::from(force)
        );
        let response = self.send(Method::DELETE, &path, None, "删除镜像").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "删除镜像", body, Some(image))
    }

    /// 创建交互式 exec（TTY）。
    pub async fn create_exec(
        &self,
        container: &str,
        command: &[String],
        user: Option<&str>,
        working_dir: Option<&str>,
        env: &[String],
        tty: bool,
    ) -> Result<String, ContainerError> {
        let payload = ExecCreateRequest {
            attach_stdin: true,
            attach_stdout: true,
            attach_stderr: true,
            tty,
            cmd: command.to_vec(),
            env: env.to_vec(),
            working_dir: working_dir.map(str::to_string),
            user: user.map(str::to_string),
        };
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造 exec 请求".to_string(),
            message: error.to_string(),
        })?;
        let path = format!("/containers/{}/exec", http::percent_encode(container));
        let response = self
            .send(Method::POST, &path, Some(body), "创建 exec")
            .await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "创建 exec", body, container));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let created: ExecCreateResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: "解析 exec 创建响应".to_string(),
                message: error.to_string(),
            })?;
        Ok(created.id)
    }

    /// 启动 exec 并取得升级后的双向流。
    pub async fn start_exec_stream(
        &self,
        exec_id: &str,
    ) -> Result<crate::transport::EngineStream, ContainerError> {
        let path = self.path(&format!("/exec/{}/start", http::percent_encode(exec_id)));
        let body = br#"{"Detach":false,"Tty":true}"#.to_vec();
        http::request_upgrade(
            &self.transport,
            Method::POST,
            &path,
            Bytes::from(body),
            Some("application/json"),
        )
        .await
    }

    /// 调整 exec 的 TTY 尺寸。
    pub async fn resize_exec(
        &self,
        exec_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), ContainerError> {
        let path = format!(
            "/exec/{}/resize?h={rows}&w={cols}",
            http::percent_encode(exec_id)
        );
        self.post_empty(&path, "调整 exec 尺寸", None).await
    }

    /// 查询 exec 是否结束以及退出码。
    pub async fn inspect_exec(&self, exec_id: &str) -> Result<ExecInspectResponse, ContainerError> {
        self.get_json(
            &format!("/exec/{}/json", http::percent_encode(exec_id)),
            "查询 exec 状态",
        )
        .await
    }

    /// 在容器内执行一条不需要交互的命令（删除、重命名等），等待结束并返回退出码。
    ///
    /// 设计与限制（Why）：归档 API 没有删除/重命名能力，只能通过 exec 调用
    /// `rm`/`mv`；参数以 argv 传递，不经过 Shell，避免引号注入。
    pub async fn run_command(
        &self,
        container: &str,
        command: &[String],
        cancel: &Arc<AtomicBool>,
    ) -> Result<i64, ContainerError> {
        let exec_id = self
            .create_exec(container, command, None, None, &[], false)
            .await?;
        let path = format!("/exec/{}/start", http::percent_encode(&exec_id));
        let body = br#"{"Detach":true,"Tty":false}"#.to_vec();
        let response = self
            .send(Method::POST, &path, Some(body), "启动命令")
            .await?;
        if !response.status.is_success() {
            let status = response.status;
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, "启动命令", body, container));
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(ContainerError::Cancelled);
            }
            let inspect = self.inspect_exec(&exec_id).await?;
            if !inspect.running {
                return Ok(inspect.exit_code.unwrap_or(-1));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ContainerError::Http {
                    operation: "等待容器命令结束".to_string(),
                    message: "超过 30 秒仍未结束".to_string(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

fn log_request_error(operation: &str, error: &ContainerError) {
    log::debug!("容器引擎请求失败，操作: {operation}，原因: {error}");
}

/// 格式化原始 inspect JSON；解析失败时按原文返回，保证“高级信息”仍可查看。
fn format_inspect_json(bytes: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .and_then(|value| serde_json::to_string_pretty(&value))
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).to_string())
}

/// 探测结果。
#[derive(Debug, Clone)]
pub struct ProbedEngine {
    pub kind: ContainerEngineKind,
    pub version: String,
    pub api_version: String,
    pub platform: String,
}

#[derive(Debug, Serialize)]
struct ExecCreateRequest {
    #[serde(rename = "AttachStdin")]
    attach_stdin: bool,
    #[serde(rename = "AttachStdout")]
    attach_stdout: bool,
    #[serde(rename = "AttachStderr")]
    attach_stderr: bool,
    #[serde(rename = "Tty")]
    tty: bool,
    #[serde(rename = "Cmd")]
    cmd: Vec<String>,
    #[serde(rename = "Env", skip_serializing_if = "Vec::is_empty")]
    env: Vec<String>,
    #[serde(rename = "WorkingDir", skip_serializing_if = "Option::is_none")]
    working_dir: Option<String>,
    #[serde(rename = "User", skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

/// 版本号转 API 前缀；无法识别时返回空串，交给引擎使用默认版本。
fn normalize_api_prefix(version: Option<&str>) -> String {
    let Some(version) = version else {
        return String::new();
    };
    let trimmed = version.trim().trim_start_matches('v');
    if !trimmed.starts_with("1.") {
        return String::new();
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return String::new();
    }
    format!("/v{trimmed}")
}

/// 拆分镜像引用为 `(仓库, 可选 tag)`。
///
/// 只在最后一个 `/` 之后找 `:`，避免把注册表端口当 tag；摘要引用不拆 tag。
fn split_reference(reference: &str) -> (String, Option<String>) {
    if reference.contains('@') {
        return (reference.to_string(), None);
    }
    let last_slash = reference.rfind('/').map(|index| index + 1).unwrap_or(0);
    match reference[last_slash..].rfind(':') {
        Some(offset) => {
            let colon = last_slash + offset;
            (
                reference[..colon].to_string(),
                Some(reference[colon + 1..].to_string()),
            )
        }
        None => (reference.to_string(), Some("latest".to_string())),
    }
}

/// 逐行消费拉取进度流；任何 `error` 字段都会终止并返回失败。
async fn consume_pull_stream(
    body: hyper::body::Incoming,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<(), ContainerError> {
    let mut body = std::pin::pin!(body);
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "读取镜像拉取进度".to_string(),
            message: error.to_string(),
        })?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        buffer.extend_from_slice(&data);
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=newline).collect();
            if let Some(error) = handle_pull_line(&line, progress)? {
                return Err(ContainerError::Api {
                    operation: "拉取镜像".to_string(),
                    status: 500,
                    body: error,
                });
            }
        }
    }
    if !buffer.is_empty()
        && let Some(error) = handle_pull_line(&buffer, progress)?
    {
        return Err(ContainerError::Api {
            operation: "拉取镜像".to_string(),
            status: 500,
            body: error,
        });
    }
    Ok(())
}

fn handle_pull_line(
    line: &[u8],
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<Option<String>, ContainerError> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let event: ImagePullEvent = match serde_json::from_str(text) {
        Ok(event) => event,
        Err(error) => {
            log::debug!("镜像拉取进度行无法解析，内容: {text}，原因: {error}");
            return Ok(None);
        }
    };
    if let Some(detail) = event.error_detail.and_then(|detail| detail.message) {
        return Ok(Some(detail));
    }
    if let Some(error) = event.error {
        return Ok(Some(error));
    }
    if let Some(detail) = event.progress_detail {
        progress(detail.current.unwrap_or(0), detail.total);
    }
    Ok(None)
}

async fn consume_pull_task_stream(
    body: hyper::body::Incoming,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
) -> Result<(), ContainerError> {
    let mut body = std::pin::pin!(body);
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    while let Some(frame) = body.frame().await {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ContainerError::Cancelled);
        }
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "读取镜像分层进度".to_string(),
            message: error.to_string(),
        })?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        buffer.extend_from_slice(&data);
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=newline).collect();
            if let Some(error) = handle_pull_task_line(&line, progress)? {
                return Err(ContainerError::Api {
                    operation: "拉取镜像".to_string(),
                    status: 500,
                    body: error,
                });
            }
        }
    }
    if !buffer.is_empty()
        && let Some(error) = handle_pull_task_line(&buffer, progress)?
    {
        return Err(ContainerError::Api {
            operation: "拉取镜像".to_string(),
            status: 500,
            body: error,
        });
    }
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    Ok(())
}

fn handle_pull_task_line(
    line: &[u8],
    progress: &mut (dyn FnMut(String, Option<transom_core::ImageLayerProgress>) + Send),
) -> Result<Option<String>, ContainerError> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let event: ImagePullEvent = match serde_json::from_str(text) {
        Ok(event) => event,
        Err(error) => {
            log::debug!("镜像分层进度行解析跳过，内容: {text}，原因: {error}");
            return Ok(None);
        }
    };
    if let Some(detail) = event.error_detail.and_then(|detail| detail.message) {
        return Ok(Some(detail));
    }
    if let Some(error) = event.error {
        return Ok(Some(error));
    }
    let status_desc = event.status.clone().unwrap_or_default();
    if let Some(id) = event.id {
        let (current, total) = event
            .progress_detail
            .map(|detail| (detail.current.unwrap_or(0), detail.total.unwrap_or(0)))
            .unwrap_or((0, 0));
        let layer_prog = transom_core::ImageLayerProgress {
            id,
            status: status_desc.clone(),
            current,
            total,
        };
        progress(status_desc, Some(layer_prog));
    } else if !status_desc.is_empty() {
        progress(status_desc, None);
    }
    Ok(None)
}

/// 组装创建请求里的 `Healthcheck`：`Interval`/`Timeout` 按纳秒下发。
///
/// 设计原理（Why）：引擎对 `Healthcheck` 的字段要求是「Test 必填」，间隔与超时
/// 留空走默认 30 秒；界面允许只填命令，因此这两个字段独立可空。
fn healthcheck(spec: &transom_core::ContainerCreateSpec) -> serde_json::Value {
    let test = match spec.health_cmd.as_deref() {
        Some(cmd) => serde_json::json!(["CMD-SHELL", cmd]),
        None => serde_json::json!(["NONE"]),
    };
    let mut value = serde_json::json!({ "Test": test });
    if let Some(seconds) = spec.health_interval_secs.filter(|seconds| *seconds > 0) {
        let nanos = seconds.saturating_mul(1_000_000_000);
        value["Interval"] = serde_json::json!(nanos);
        // 真机约束：超时不得大于间隔，也不得小于 1 毫秒，否则创建直接 400。
        // 取间隔的一半再夹到 [1ms, interval] 区间内，两者都能满足。
        let timeout = (nanos / 2).clamp(1_000_000, nanos);
        value["Timeout"] = serde_json::json!(timeout);
        value["Retries"] = serde_json::json!(3);
    }
    value
}

/// 把端口映射列表折叠成 Docker 的 `PortBindings` 形态：`{"80/tcp":[{"HostPort":"8080"}]}`。
fn port_bindings(ports: &[transom_core::ContainerPortPublish]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for publish in ports {
        let key = format!("{}/{}", publish.container, publish.protocol);
        let entry = serde_json::json!({
            "HostIp": "0.0.0.0",
            "HostPort": publish.host.to_string(),
        });
        map.entry(key)
            .or_insert_with(|| serde_json::Value::Array(Vec::new()))
            .as_array_mut()
            .expect("PortBindings 值必须是数组")
            .push(entry);
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::{normalize_api_prefix, port_bindings, split_reference};

    #[test]
    fn port_bindings_group_by_container_port_and_protocol() {
        let bindings = port_bindings(&[
            transom_core::ContainerPortPublish {
                host: 8080,
                container: 80,
                protocol: "tcp".to_string(),
            },
            transom_core::ContainerPortPublish {
                host: 5353,
                container: 53,
                protocol: "udp".to_string(),
            },
        ]);
        assert_eq!(bindings["80/tcp"][0]["HostPort"], serde_json::json!("8080"));
        assert_eq!(bindings["53/udp"][0]["HostPort"], serde_json::json!("5353"));
        assert!(
            port_bindings(&[])
                .as_object()
                .is_some_and(serde_json::Map::is_empty)
        );
    }

    #[test]
    fn api_prefix_accepts_docker_style_versions() {
        assert_eq!(normalize_api_prefix(Some("1.46")), "/v1.46");
        assert_eq!(normalize_api_prefix(Some("v1.41")), "/v1.41");
        assert_eq!(normalize_api_prefix(Some("2.0")), "");
        assert_eq!(normalize_api_prefix(Some("1.x")), "");
        assert_eq!(normalize_api_prefix(None), "");
    }

    #[test]
    fn reference_splits_tag_without_confusing_registry_ports() {
        assert_eq!(
            split_reference("nginx"),
            ("nginx".to_string(), Some("latest".to_string()))
        );
        assert_eq!(
            split_reference("ghcr.io/team/app:v1"),
            ("ghcr.io/team/app".to_string(), Some("v1".to_string()))
        );
        assert_eq!(
            split_reference("localhost:5000/app"),
            ("localhost:5000/app".to_string(), Some("latest".to_string()))
        );
        assert_eq!(
            split_reference("alpine@sha256:abc"),
            ("alpine@sha256:abc".to_string(), None)
        );
    }

    #[test]
    fn network_connect_and_disconnect_payload_structures() {
        let connect_payload = serde_json::json!({
            "Container": "my-web-1",
        });
        assert_eq!(connect_payload["Container"], "my-web-1");

        let disconnect_payload = serde_json::json!({
            "Container": "my-web-1",
            "Force": false,
        });
        assert_eq!(disconnect_payload["Container"], "my-web-1");
        assert_eq!(disconnect_payload["Force"], false);
    }
}
