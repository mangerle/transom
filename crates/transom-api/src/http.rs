//! 本地桌面 HTTP 执行服务：全局复用 `reqwest::Client` 连接池，并限制响应体大小。

use super::{ApiHeader, ApiRequest};
use futures::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, Method, Url};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 单次响应体上限；超出时停止读取，避免未知体量响应把界面内存打满。
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// API 请求构建、传输与读取阶段的强类型错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApiError {
    #[error("请求地址无效，目标: {host}，原因: {reason}")]
    InvalidUrl { host: String, reason: String },
    #[error("请求头格式无效，行号: {line}，原因: {reason}")]
    InvalidHeader { line: usize, reason: String },
    #[error("请求头 {name} 的值无效，原因: {reason}")]
    InvalidHeaderValue { name: String, reason: String },
    #[error("发送请求失败，目标: {host}，原因: {reason}")]
    Transport { host: String, reason: String },
    #[error("读取响应失败，目标: {host}，原因: {reason}")]
    ResponseRead { host: String, reason: String },
    #[error("响应体超过 {limit} 字节上限，目标: {host}")]
    ResponseTooLarge { host: String, limit: usize },
    #[error("解析响应文本失败，目标: {host}，原因: {reason}")]
    ResponseText { host: String, reason: String },
}

/// 可直接展示的响应快照；不保存请求头或请求体，避免 UI 重绘时重复持有敏感输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponseSnapshot {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub body_size: usize,
    pub elapsed_millis: u128,
}

/// 全局唯一的 API HTTP 客户端服务。
#[derive(Clone)]
pub struct ApiHttpService {
    client: Client,
    runtime: Handle,
}

/// 一次在 Tokio 控制面执行的请求；接收端可在 GPUI 执行器上等待。
///
/// 设计原理（Why）：GPUI 的异步执行器不是 Tokio reactor，直接把 `reqwest`
/// Future 放进 GPUI 任务会触发“没有运行中的 reactor”并崩溃。请求必须在应用
/// 已有的 Tokio Runtime 中执行，结果经容量为 1 的 `oneshot` 返回 UI；Drop 时
/// 中止 Tokio 任务，关闭标签或重新发送都能取消旧请求。
pub struct ApiExchange {
    receiver: oneshot::Receiver<Result<ApiResponseSnapshot, ApiError>>,
    task: JoinHandle<()>,
}

impl ApiHttpService {
    /// 构造长生命周期 HTTP 客户端；TLS 初始化失败时返回明确错误。
    ///
    /// # Errors
    /// `reqwest::Client` 构建失败时返回 `ApiError::Transport`。
    pub fn new(runtime: Handle) -> Result<Self, ApiError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ApiError::Transport {
                host: "HTTP 客户端".to_string(),
                reason: error.to_string(),
            })?;
        Ok(Self { client, runtime })
    }

    /// 在 Tokio 控制面发起请求，返回可由 UI 等待的有界一次性结果。
    pub fn start(&self, request: ApiRequest) -> ApiExchange {
        let client = self.client.clone();
        let (sender, receiver) = oneshot::channel();
        let task = self.runtime.spawn(async move {
            let result = execute_request(&client, &request).await;
            let _ = sender.send(result);
        });
        ApiExchange { receiver, task }
    }

    /// 发送请求并读取受限大小的响应体。
    ///
    /// # Errors
    /// URL、请求头、网络传输或响应读取失败时返回 [`ApiError`]。
    pub async fn execute(&self, request: &ApiRequest) -> Result<ApiResponseSnapshot, ApiError> {
        execute_request(&self.client, request).await
    }
}

impl ApiExchange {
    /// 等待 Tokio 控制面返回结果。
    ///
    /// # Errors
    /// 请求构造、传输、响应读取失败，或执行任务在返回前被中止时返回
    /// [`ApiError`]。
    pub async fn recv(&mut self) -> Result<ApiResponseSnapshot, ApiError> {
        match (&mut self.receiver).await {
            Ok(result) => result,
            Err(_) => Err(ApiError::Transport {
                host: "API 请求任务".to_string(),
                reason: "执行任务在返回结果前结束".to_string(),
            }),
        }
    }
}

impl Drop for ApiExchange {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn execute_request(
    client: &Client,
    request: &ApiRequest,
) -> Result<ApiResponseSnapshot, ApiError> {
    let url = Url::parse(request.url.trim()).map_err(|error| ApiError::InvalidUrl {
        host: safe_url_label(request.url.trim()),
        reason: error.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::InvalidUrl {
            host: safe_url_label(request.url.trim()),
            reason: "只支持 http 与 https 地址".to_string(),
        });
    }
    let host = request_host(&url);
    let method = method_for(request.method);
    let mut builder = client.request(method, url).body(request.body.clone());
    for header in request.headers.iter().filter(|header| header.enabled) {
        builder = builder.header(header_name(header)?, header_value(header)?);
    }

    let started = Instant::now();
    let response = builder.send().await.map_err(|error| ApiError::Transport {
        host: host.clone(),
        reason: error.to_string(),
    })?;
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or("<不可显示>").to_string(),
            )
        })
        .collect::<Vec<_>>();
    let body = read_limited_body(response, &host).await?;
    let body_size = body.len();
    let body = display_body(body, &host)?;
    Ok(ApiResponseSnapshot {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
        body_size,
        elapsed_millis: started.elapsed().as_millis(),
    })
}

fn method_for(method: super::ApiMethod) -> Method {
    match method {
        super::ApiMethod::Get => Method::GET,
        super::ApiMethod::Post => Method::POST,
        super::ApiMethod::Put => Method::PUT,
        super::ApiMethod::Patch => Method::PATCH,
        super::ApiMethod::Delete => Method::DELETE,
        super::ApiMethod::Head => Method::HEAD,
        super::ApiMethod::Options => Method::OPTIONS,
    }
}

fn header_name(header: &ApiHeader) -> Result<HeaderName, ApiError> {
    HeaderName::from_bytes(header.name.trim().as_bytes()).map_err(|error| ApiError::InvalidHeader {
        line: 0,
        reason: format!("名称 {} 无效: {error}", header.name),
    })
}

fn header_value(header: &ApiHeader) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(header.value.as_str()).map_err(|error| ApiError::InvalidHeaderValue {
        name: header.name.clone(),
        reason: error.to_string(),
    })
}

async fn read_limited_body(response: reqwest::Response, host: &str) -> Result<Vec<u8>, ApiError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ApiError::ResponseRead {
            host: host.to_string(),
            reason: error.to_string(),
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ApiError::ResponseTooLarge {
                host: host.to_string(),
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn display_body(body: Vec<u8>, host: &str) -> Result<String, ApiError> {
    let text = String::from_utf8(body).map_err(|error| ApiError::ResponseText {
        host: host.to_string(),
        reason: error.to_string(),
    })?;
    let trimmed = text.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        serde_json::from_str::<serde_json::Value>(trimmed)
            .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.clone()))
            .or(Ok(text))
    } else {
        Ok(text)
    }
}

fn request_host(url: &Url) -> String {
    let mut label = format!(
        "{}://{}",
        url.scheme(),
        url.host_str().unwrap_or("未知主机")
    );
    if let Some(port) = url.port() {
        label.push_str(&format!(":{port}"));
    }
    label.push_str(url.path());
    label
}

fn safe_url_label(input: &str) -> String {
    Url::parse(input)
        .map(|url| request_host(&url))
        .unwrap_or_else(|_| "无法解析的地址".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn executes_get_against_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.expect("读取请求");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\n\r\n{\"ok\":true}\n")
                .await
                .expect("写响应");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let request = ApiRequest {
            id: super::super::ApiRequestId::new(),
            folder_id: None,
            name: "测试".to_string(),
            method: super::super::ApiMethod::Get,
            url: format!("http://{address}/health"),
            headers: Vec::new(),
            body: String::new(),
        };
        let response = service.execute(&request).await.expect("请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 200);
        assert!(response.body.contains("\"ok\": true"));
    }
}
