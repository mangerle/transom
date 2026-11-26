//! 本地 API HTTP、GraphQL 与原始 gRPC 执行服务。

use super::{ApiAuth, ApiCookie, ApiEnvironment, ApiExecutionPlan, ApiProtocol, ApiRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::{Client, Method, Url, Version, redirect::Policy};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use self::http_auth::{AuthRequest, apply_auth};
use super::http_body::{PreparedRequestParts, prepared_http_parts, send_form_http};
use super::http_support::{
    header_name, header_value, method_for, read_grpc_response, read_response, request_host,
    safe_url_label,
};
use crate::model::cookie_header_value;

#[path = "http/http_auth.rs"]
pub(super) mod http_auth;

/// 单次响应体上限；超出时停止读取，避免未知体量响应把界面内存打满。
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// 单个上传文件上限；UI 预览与发送执行共用同一阈值。
pub const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MAX_REQUEST_TIMEOUT_SECONDS: u64 = 600;

/// 可直接展示的响应快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponseSnapshot {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub body_size: usize,
    pub elapsed_millis: u128,
    pub set_cookies: Vec<ApiCookie>,
}

/// API 请求构建、传输与读取阶段的强类型错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApiError {
    #[error("请求地址无效，目标: {host}，原因: {reason}")]
    InvalidUrl { host: String, reason: String },
    #[error("请求头格式无效，行号: {line}，原因: {reason}")]
    InvalidHeader { line: usize, reason: String },
    #[error("请求头 {name} 的值无效，原因: {reason}")]
    InvalidHeaderValue { name: String, reason: String },
    #[error("认证配置无效，原因: {reason}")]
    InvalidAuth { reason: String },
    #[error("断言第 {line} 行无效，原因: {reason}")]
    InvalidAssertion { line: usize, reason: String },
    #[error("OAuth 2.0 获取令牌失败，地址: {url}，原因: {reason}")]
    OAuthToken { url: String, reason: String },
    #[error("GraphQL 变量无效，原因: {reason}")]
    InvalidGraphQlVariables { reason: String },
    #[error("gRPC 请求体不是有效 Base64，原因: {reason}")]
    InvalidGrpcPayload { reason: String },
    #[error("请求体配置无效，原因: {reason}")]
    InvalidBody { reason: String },
    #[error("协议 {protocol} 配置无效，原因: {reason}")]
    InvalidProtocolConfig { protocol: String, reason: String },
    #[error("当前执行器不支持协议 {protocol}")]
    UnsupportedProtocol { protocol: String },
    #[error("发送请求失败，目标: {host}，原因: {reason}")]
    Transport { host: String, reason: String },
    #[error("API 请求任务异常退出，原因: {reason}")]
    TaskFailed { reason: String },
    #[error("读取响应失败，目标: {host}，原因: {reason}")]
    ResponseRead { host: String, reason: String },
    #[error("响应体超过 {limit} 字节上限，目标: {host}")]
    ResponseTooLarge { host: String, limit: usize },
    #[error("读取上传文件失败，路径: {path}，原因: {reason}")]
    UploadFile { path: String, reason: String },
    #[error("上传文件超过 {limit} 字节上限，路径: {path}")]
    UploadFileTooLarge { path: String, limit: u64 },
    #[error("构建 multipart 文件字段失败，字段: {field}，原因: {reason}")]
    MultipartField { field: String, reason: String },
}

/// 一次在 Tokio 控制面执行的请求；接收端可在 GPUI 执行器上等待。
pub struct ApiExchange {
    receiver: oneshot::Receiver<Result<ApiResponseSnapshot, ApiError>>,
    task: Option<JoinHandle<()>>,
}

/// 全局唯一的 API HTTP 客户端服务。
#[derive(Clone)]
pub struct ApiHttpService {
    client: Client,
    token_client: Client,
    runtime: Handle,
}

impl ApiHttpService {
    /// 构造长生命周期 HTTP 客户端；TLS/HTTP2 初始化失败时返回明确错误。
    ///
    /// # Errors
    /// `reqwest::Client` 构建失败时返回 `ApiError::Transport`。
    pub fn new(runtime: Handle) -> Result<Self, ApiError> {
        let client = build_client(Policy::limited(10))?;
        let token_client = build_client(Policy::none())?;
        Ok(Self {
            client,
            token_client,
            runtime,
        })
    }

    /// 在 Tokio 控制面执行已解析请求，返回可由 UI 等待的一次性结果。
    pub fn start(&self, plan: ApiExecutionPlan) -> ApiExchange {
        let client = self.client.clone();
        let token_client = self.token_client.clone();
        let (sender, receiver) = oneshot::channel();
        let task = self.runtime.spawn(async move {
            let result = execute_request(&client, &token_client, &plan).await;
            let _ = sender.send(result);
        });
        ApiExchange {
            receiver,
            task: Some(task),
        }
    }

    /// 直接执行请求；测试和 Tokio 侧调用使用。
    ///
    /// # Errors
    /// URL、认证、网络或响应读取失败时返回 [`ApiError`]。
    pub async fn execute(&self, plan: &ApiExecutionPlan) -> Result<ApiResponseSnapshot, ApiError> {
        execute_request(&self.client, &self.token_client, plan).await
    }
}

fn build_client(redirect_policy: Policy) -> Result<Client, ApiError> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(redirect_policy)
        .build()
        .map_err(|error| ApiError::Transport {
            host: "HTTP 客户端".to_string(),
            reason: error.to_string(),
        })
}

impl ApiExchange {
    /// 等待 Tokio 控制面返回结果。
    ///
    /// # Errors
    /// 请求构造、传输、响应读取失败，或执行任务在返回前中止时返回
    /// [`ApiError`]。
    pub async fn recv(&mut self) -> Result<ApiResponseSnapshot, ApiError> {
        let receiver_result = (&mut self.receiver).await;
        if let Some(task) = self.task.take() {
            task.await.map_err(|error| ApiError::TaskFailed {
                reason: error.to_string(),
            })?;
        }
        match receiver_result {
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
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn execute_request(
    client: &Client,
    token_client: &Client,
    plan: &ApiExecutionPlan,
) -> Result<ApiResponseSnapshot, ApiError> {
    let request = plan.request();
    let environment = plan.environment();
    let cookies = plan.cookies();
    match request.protocol {
        ApiProtocol::Http => {
            send_http(client, token_client, request, environment, cookies, false).await
        }
        ApiProtocol::GraphQl => {
            send_http(client, token_client, request, environment, cookies, true).await
        }
        ApiProtocol::Grpc => send_grpc(client, token_client, request, environment).await,
        ApiProtocol::WebSocket => Err(ApiError::UnsupportedProtocol {
            protocol: ApiProtocol::WebSocket.as_str().to_string(),
        }),
    }
}

async fn send_http(
    client: &Client,
    token_client: &Client,
    request: &ApiRequest,
    environment: &ApiEnvironment,
    cookies: &[ApiCookie],
    graphql: bool,
) -> Result<ApiResponseSnapshot, ApiError> {
    if !graphql && request.body_type.is_form() {
        return send_form_http(client, token_client, request, environment, cookies).await;
    }
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
    let method = if graphql {
        Method::POST
    } else {
        method_for(request.method)
    };
    let PreparedRequestParts { mut headers, body } = prepared_http_parts(request, graphql).await?;
    apply_cookies(&url, cookies, &mut headers);
    let payload_hash = if matches!(request.auth, ApiAuth::AwsSigV4 { .. }) {
        body.sha256_hex().await?
    } else {
        None
    };
    apply_auth(AuthRequest {
        token_client,
        request,
        environment,
        headers: &mut headers,
        body: body.as_bytes().unwrap_or(&[]),
        payload_hash: payload_hash.as_deref(),
        url: &url,
        method: method.as_str(),
    })
    .await?;
    let request_body = body.into_reqwest_body().await?;
    let mut builder = client.request(method, url.clone()).body(request_body);
    if request.timeout_seconds > 0 {
        builder = builder.timeout(Duration::from_secs(
            request.timeout_seconds.min(MAX_REQUEST_TIMEOUT_SECONDS),
        ));
    }
    for header in &headers {
        builder = builder.header(header_name(header)?, header_value(header)?);
    }
    let started = Instant::now();
    let response = builder.send().await.map_err(|error| ApiError::Transport {
        host: host.clone(),
        reason: error.to_string(),
    })?;
    read_response(response, started, &host).await
}

async fn send_grpc(
    client: &Client,
    token_client: &Client,
    request: &ApiRequest,
    environment: &ApiEnvironment,
) -> Result<ApiResponseSnapshot, ApiError> {
    if request.grpc_service.trim().is_empty() || request.grpc_method.trim().is_empty() {
        return Err(ApiError::InvalidProtocolConfig {
            protocol: ApiProtocol::Grpc.as_str().to_string(),
            reason: "gRPC 服务名和方法名不能为空".to_string(),
        });
    }
    let base = request.url.trim();
    let url = Url::parse(base).map_err(|error| ApiError::InvalidUrl {
        host: safe_url_label(base),
        reason: error.to_string(),
    })?;
    let host = request_host(&url);
    let mut endpoint = url.clone();
    endpoint.set_path(&format!(
        "/{}/{}",
        request.grpc_service.trim().trim_matches('/'),
        request.grpc_method.trim().trim_matches('/')
    ));
    let payload = if request.body.trim().is_empty() {
        Vec::new()
    } else {
        STANDARD
            .decode(request.body.trim())
            .map_err(|error| ApiError::InvalidGrpcPayload {
                reason: error.to_string(),
            })?
    };
    let mut framed = Vec::with_capacity(payload.len().saturating_add(5));
    framed.extend_from_slice(&[0, 0, 0, 0, 0]);
    let length = u32::try_from(payload.len()).map_err(|_| ApiError::InvalidGrpcPayload {
        reason: "gRPC 请求体长度超过 u32".to_string(),
    })?;
    framed[1..5].copy_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(&payload);
    let mut headers = vec![
        ("content-type".to_string(), "application/grpc".to_string()),
        ("te".to_string(), "trailers".to_string()),
        ("grpc-accept-encoding".to_string(), "identity".to_string()),
    ];
    for header in request.headers.iter().filter(|header| header.enabled) {
        headers.push((header.name.clone(), header.value.clone()));
    }
    apply_auth(AuthRequest {
        token_client,
        request,
        environment,
        headers: &mut headers,
        body: &framed,
        payload_hash: None,
        url: &endpoint,
        method: "POST",
    })
    .await?;
    let mut builder = client
        .post(endpoint.clone())
        .version(Version::HTTP_2)
        .body(framed);
    if request.timeout_seconds > 0 {
        builder = builder.timeout(Duration::from_secs(
            request.timeout_seconds.min(MAX_REQUEST_TIMEOUT_SECONDS),
        ));
    }
    for header in &headers {
        builder = builder.header(header_name(header)?, header_value(header)?);
    }
    let started = Instant::now();
    let response = builder.send().await.map_err(|error| ApiError::Transport {
        host: host.clone(),
        reason: error.to_string(),
    })?;
    read_grpc_response(response, started, &host).await
}

pub(super) fn apply_cookies(url: &Url, cookies: &[ApiCookie], headers: &mut Vec<(String, String)>) {
    if let Some(value) = cookie_header_value(url, cookies) {
        if let Some((_, existing)) = headers
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        {
            if !existing.is_empty() {
                existing.push_str("; ");
            }
            existing.push_str(&value);
        } else {
            headers.push(("cookie".to_string(), value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApiAuth;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn plan(
        request: ApiRequest,
        environment: ApiEnvironment,
        cookies: Vec<ApiCookie>,
    ) -> ApiExecutionPlan {
        ApiExecutionPlan::new(request, environment, cookies).expect("测试执行计划应合法")
    }

    #[tokio::test]
    async fn exchange_reports_panicked_background_task() {
        let (sender, receiver) = oneshot::channel();
        drop(sender);
        let task = tokio::spawn(async {
            panic!("模拟请求任务 panic");
        });
        let mut exchange = ApiExchange {
            receiver,
            task: Some(task),
        };
        let error = exchange.recv().await.expect_err("任务 panic 应返回错误");
        assert!(matches!(error, ApiError::TaskFailed { .. }));
    }

    #[tokio::test]
    async fn executes_get_against_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut request = [0_u8; 1024];
            let read = stream.read(&mut request).await.expect("读取请求");
            let text = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            assert!(text.contains("cookie: session=abc"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: theme=dark; Path=/; HttpOnly\r\nContent-Length: 12\r\n\r\n{\"ok\":true}\n")
                .await
                .expect("写响应");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let request = ApiRequest {
            id: super::super::ApiRequestId::new(),
            folder_id: None,
            name: "测试".to_string(),
            protocol: ApiProtocol::Http,
            method: super::super::ApiMethod::Get,
            url: format!("http://{address}/health"),
            description: String::new(),
            timeout_seconds: 0,
            query: Vec::new(),
            headers: Vec::new(),
            body_type: super::super::ApiBodyType::Text,
            raw_language: super::super::ApiRawLanguage::Text,
            body: String::new(),
            form_fields: Vec::new(),
            body_file: String::new(),
            graphql_variables: "{}".to_string(),
            grpc_service: String::new(),
            grpc_method: String::new(),
            auth: ApiAuth::None,
            assertions: Vec::new(),
        };
        let cookie = ApiCookie {
            name: "session".to_string(),
            value: "abc".to_string(),
            domain: None,
            path: "/".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        };
        let response = service
            .execute(&plan(request, ApiEnvironment::empty(), vec![cookie]))
            .await
            .expect("请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 200);
        assert!(response.body.contains("\"ok\": true"));
        assert_eq!(response.set_cookies[0].name, "theme");
        assert_eq!(response.set_cookies[0].value, "dark");
    }

    #[tokio::test]
    async fn decodes_gzip_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.expect("读取请求");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: 31\r\nConnection: close\r\n\r\n")
                .await
                .expect("写响应头");
            stream
                .write_all(&[
                    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xab, 0x56, 0xca,
                    0xcf, 0x56, 0xb2, 0x2a, 0x29, 0x2a, 0x4d, 0xad, 0x05, 0x00, 0x90, 0x5f, 0xd4,
                    0xa7, 0x0b, 0x00, 0x00, 0x00,
                ])
                .await
                .expect("写压缩响应体");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let mut request = ApiRequest::new("压缩响应", None);
        request.url = format!("http://{address}/compressed");
        let response = service
            .execute(&plan(request, ApiEnvironment::empty(), Vec::new()))
            .await
            .expect("gzip 响应应可解码");
        server.await.expect("测试服务结束");
        assert!(response.body.contains("\"ok\": true"));
    }

    #[tokio::test]
    async fn rejects_oversized_response_before_reading_body() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.expect("读取请求");
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        MAX_RESPONSE_BYTES.saturating_add(1)
                    )
                    .as_bytes(),
                )
                .await
                .expect("写响应头");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let mut request = ApiRequest::new("超限响应", None);
        request.url = format!("http://{address}/large");
        let error = service
            .execute(&plan(request, ApiEnvironment::empty(), Vec::new()))
            .await
            .expect_err("超限响应应被拒绝");
        server.await.expect("测试服务结束");
        assert!(matches!(error, ApiError::ResponseTooLarge { .. }));
    }

    #[tokio::test]
    async fn executes_graphql_request_as_json_post() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut request = vec![0_u8; 4096];
            let read = stream.read(&mut request).await.expect("读取请求");
            let text = String::from_utf8_lossy(&request[..read]);
            assert!(text.contains("POST"));
            assert!(text.contains("query"));
            assert!(text.contains("query Health"));
            assert!(text.contains("variables"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}")
                .await
                .expect("写响应");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let request = ApiRequest::new("GraphQL", None);
        let mut request = request;
        request.protocol = ApiProtocol::GraphQl;
        request.url = format!("http://{address}/graphql");
        request.body = "query {{OPERATION}}".to_string();
        let mut environment = ApiEnvironment::empty();
        environment.variables.push(crate::ApiVariable {
            name: "OPERATION".to_string(),
            value: "Health".to_string(),
            enabled: true,
            secret: false,
        });
        let response = service
            .execute(&plan(
                request.resolved("", &environment),
                environment,
                Vec::new(),
            ))
            .await
            .expect("GraphQL 请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn sends_form_urlencoded_body() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut buffer = [0_u8; 2048];
            let read = stream.read(&mut buffer).await.expect("读取请求");
            let text = String::from_utf8_lossy(&buffer[..read]);
            assert!(text.contains("name=Alice"));
            assert!(text.contains("city=New+York"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK
Content-Length: 2

{}",
                )
                .await
                .expect("写响应");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let mut request = ApiRequest::new("表单", None);
        request.method = super::super::ApiMethod::Post;
        request.url = format!("http://{address}/form");
        request.body_type = super::super::ApiBodyType::FormUrlEncoded;
        request.form_fields = vec![
            super::super::ApiFormField::new("name", "Alice"),
            super::super::ApiFormField::new("city", "New York"),
        ];
        let response = service
            .execute(&plan(request, ApiEnvironment::empty(), Vec::new()))
            .await
            .expect("表单请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn sends_multipart_file_body() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("hello.txt");
        std::fs::write(&path, "hello upload").expect("写测试文件");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受连接");
            let mut received = Vec::with_capacity(8192);
            let mut chunk = [0_u8; 8192];
            let text = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let read = stream.read(&mut chunk).await.expect("读取请求");
                    if read == 0 {
                        break;
                    }
                    received.extend_from_slice(&chunk[..read]);
                    if String::from_utf8_lossy(&received).contains("hello upload") {
                        break;
                    }
                }
                String::from_utf8_lossy(&received).into_owned()
            })
            .await
            .expect("读取 multipart 请求超时");
            assert!(text.contains("multipart/form-data"));
            assert!(text.contains("name=\"file\""));
            assert!(text.contains("filename=\"hello.txt\""));
            assert!(text.contains("hello upload"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK
Content-Length: 2

{}",
                )
                .await
                .expect("写响应");
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let mut request = ApiRequest::new("文件", None);
        request.method = super::super::ApiMethod::Post;
        request.url = format!("http://{address}/upload");
        request.body_type = super::super::ApiBodyType::Multipart;
        request.form_fields = vec![super::super::ApiFormField {
            name: "file".to_string(),
            value: format!("file://{}", path.display()),
            description: String::new(),
            enabled: true,
            kind: super::super::ApiFormFieldKind::File,
        }];
        let response = service
            .execute(&plan(request, ApiEnvironment::empty(), Vec::new()))
            .await
            .expect("multipart 请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn obtains_oauth_token_and_applies_bearer_header() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受令牌请求");
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).await.expect("读取令牌请求");
            let text = String::from_utf8_lossy(&request[..read]);
            assert!(text.starts_with("POST /token HTTP/1.1"));
            assert!(text.contains("grant_type=client_credentials"));
            let body = "{\"access_token\":\"test-token\"}";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .expect("写令牌响应");

            let (mut stream, _) = listener.accept().await.expect("接受业务请求");
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).await.expect("读取业务请求");
            let text = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            assert!(text.starts_with("get /resource http/1.1"));
            assert!(text.contains("authorization: bearer test-token"));
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")
                .await
                .expect("写业务响应");
        });

        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let mut request = ApiRequest::new("OAuth", None);
        request.url = format!("http://{address}/resource");
        request.auth = ApiAuth::OAuth2ClientCredentials {
            token_url: format!("http://{address}/token"),
            client_id: "client-id".to_string(),
            client_secret_env: "CLIENT_SECRET".to_string(),
            scope: "read".to_string(),
        };
        let mut environment = ApiEnvironment::empty();
        environment.variables.push(crate::ApiVariable {
            name: "CLIENT_SECRET".to_string(),
            value: "secret".to_string(),
            enabled: true,
            secret: true,
        });
        let response = service
            .execute(&plan(request, environment, Vec::new()))
            .await
            .expect("OAuth 请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 201);
    }

    #[tokio::test]
    async fn oauth_token_client_does_not_follow_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("接受令牌请求");
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).await.expect("读取令牌请求");
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST /token HTTP/1.1"));
            stream
                .write_all(
                    b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /leak\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .expect("写入重定向响应");
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "OAuth 令牌客户端不应跟随重定向"
            );
        });
        let service = ApiHttpService::new(tokio::runtime::Handle::current()).expect("构造服务");
        let mut request = ApiRequest::new("OAuth 重定向", None);
        request.url = format!("http://{address}/resource");
        request.auth = ApiAuth::OAuth2ClientCredentials {
            token_url: format!("http://{address}/token"),
            client_id: "client-id".to_string(),
            client_secret_env: "CLIENT_SECRET".to_string(),
            scope: "read".to_string(),
        };
        let mut environment = ApiEnvironment::empty();
        environment.variables.push(crate::ApiVariable {
            name: "CLIENT_SECRET".to_string(),
            value: "secret".to_string(),
            enabled: true,
            secret: true,
        });
        let error = service
            .execute(&plan(request, environment, Vec::new()))
            .await
            .expect_err("重定向应被拒绝");
        assert!(matches!(error, ApiError::OAuthToken { .. }));
        assert!(error.to_string().contains("307"));
        server.await.expect("测试服务结束");
    }
}
