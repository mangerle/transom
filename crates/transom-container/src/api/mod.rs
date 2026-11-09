//! Docker 兼容 REST 客户端。
//!
//! Docker 与 Podman 在本项目用到的端点（列表、检视、启停、exec、日志、
//! 镜像、归档）上保持兼容，因此只实现一套协议；Podman 的 `/libpod` 专有
//! 接口不在范围内。
//!
//! 客户端按资源域拆到子模块，共享的请求管线（发送、JSON 解码、版本前缀）
//! 留在本模块；子模块只写各自端点的编排逻辑。

use crate::error::ContainerError;
use crate::http::{self, ConnectionPool, RawResponse};
use crate::models::VersionResponse;
use crate::transport::EngineTransport;
use bytes::Bytes;
use hyper::Method;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use transom_core::ContainerEngineKind;

/// 小体积 JSON 响应的上限：超过视为异常响应，避免误读巨型对象。
const JSON_BODY_LIMIT: usize = 8 * 1024 * 1024;

/// 版本协商后的 API 客户端。
#[derive(Debug, Clone)]
pub struct DockerApi {
    transport: EngineTransport,
    /// 形如 `/v1.46`；协商不出可用版本时为空串（走引擎的默认版本）。
    prefix: String,
    /// 普通 GET 请求的空闲连接池；克隆客户端时共享同一份池。
    pool: Arc<ConnectionPool>,
}

mod containers;
mod exec;
mod images;
mod networks;
mod system;
mod volumes;

impl DockerApi {
    pub fn new(transport: EngineTransport, api_version: Option<&str>) -> Self {
        Self {
            transport,
            prefix: normalize_api_prefix(api_version),
            pool: Arc::new(ConnectionPool::default()),
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
        let http::PooledResponse {
            status: ping_status,
            headers,
            body,
            mut lease,
        } = http::pooled_request(&self.pool, &self.transport, Method::GET, "/_ping").await?;
        let ping_header = http::header_str(&headers, "API-Version");
        let ping_body =
            String::from_utf8_lossy(&http::read_body_bytes(body, 1024).await?).to_string();
        if !ping_status.is_success() {
            return Err(ContainerError::Api {
                operation: "探测引擎 /_ping".to_string(),
                status: ping_status.as_u16(),
                body: ping_body,
            });
        }
        // /_ping 与 /version 都是普通 GET；响应体读完后立即归还，让版本协商
        // 与后续列表复用同一条连接。
        lease.mark_reusable();

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
            pool: self.pool,
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
        let path = self.path(suffix);
        let http::PooledResponse {
            status,
            body,
            mut lease,
            ..
        } = http::pooled_request(&self.pool, &self.transport, Method::GET, &path)
            .await
            .inspect_err(|error| log_request_error(operation, error))?;
        if !status.is_success() {
            let body = http::read_body_text(body, 8192).await;
            return Err(http::status_error(status, operation, body, suffix));
        }
        let bytes = http::read_body_bytes(body, JSON_BODY_LIMIT).await?;
        // 响应体完整读完，连接可以安全复用。
        lease.mark_reusable();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_prefix_accepts_docker_style_versions() {
        assert_eq!(normalize_api_prefix(Some("1.46")), "/v1.46");
        assert_eq!(normalize_api_prefix(Some("v1.41")), "/v1.41");
        assert_eq!(normalize_api_prefix(Some("2.0")), "");
        assert_eq!(normalize_api_prefix(Some("1.x")), "");
        assert_eq!(normalize_api_prefix(None), "");
    }
}
