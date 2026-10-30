//! Docker 兼容 REST 客户端。
//!
//! Docker 与 Podman 在本项目用到的端点（列表、检视、启停、exec、日志、
//! 镜像、归档）上保持兼容，因此只实现一套协议；Podman 的 `/libpod` 专有
//! 接口不在范围内。

use crate::error::ContainerError;
use crate::http::{self, RawResponse};
use crate::models::{
    ContainerInspect, ContainerListItem, ExecCreateResponse, ExecInspectResponse, ImageInspect,
    ImageListItem, ImagePullEvent, NetworkItem, StatsResponse, VersionResponse, VolumeItem,
    VolumeListResponse,
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
    ContainerDetails, ContainerImageDetails, ContainerImageSummary, ContainerLifecycleAction,
    ContainerNetworkDetails, ContainerNetworkSummary, ContainerStats, ContainerSummary,
    ContainerVolumeDetails, ContainerVolumeSummary,
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
        let suffix = format!("/containers/{}/json", http::percent_encode(container));
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

    /// 镜像详情。
    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<ContainerImageDetails, ContainerError> {
        let inspect: ImageInspect = self
            .get_json(
                &format!("/images/{}/json", http::percent_encode(image)),
                "检视镜像",
            )
            .await?;
        Ok(inspect.into_details())
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
        let volume: VolumeItem = self
            .get_json(
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
        })
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
        let item: NetworkItem = self
            .get_json(
                &format!("/networks/{}", http::percent_encode(network)),
                "检视网络",
            )
            .await?;
        Ok(item.into_details())
    }

    pub async fn create_network(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        let payload = serde_json::json!({
            "Name": name,
            "Driver": driver.unwrap_or("bridge"),
            "CheckDuplicate": true,
        });
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

#[cfg(test)]
mod tests {
    use super::{normalize_api_prefix, split_reference};

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
}
