//! 本地 API HTTP、GraphQL 与原始 gRPC 执行服务。

use super::{ApiAuth, ApiCookie, ApiEnvironment, ApiProtocol, ApiRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use reqwest::{Client, Method, Url, Version};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::http_support::{
    ensure_header, grpc_payload, header_name, header_value, method_for, read_response,
    request_host, safe_url_label,
};

/// 单次响应体上限；超出时停止读取，避免未知体量响应把界面内存打满。
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REQUEST_TIMEOUT_SECONDS: u64 = 600;

/// 认证阶段需要的完整请求上下文；集中传递避免参数列表继续膨胀。
struct AuthRequest<'a> {
    client: &'a Client,
    request: &'a ApiRequest,
    environment: &'a ApiEnvironment,
    headers: &'a mut Vec<(String, String)>,
    body: &'a [u8],
    url: &'a Url,
    method: &'a str,
}

/// HTTP 或 GraphQL 请求在应用认证与 Cookie 前的已编码部分。
struct PreparedRequestParts {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

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
    #[error("当前执行器不支持协议 {protocol}")]
    UnsupportedProtocol { protocol: String },
    #[error("发送请求失败，目标: {host}，原因: {reason}")]
    Transport { host: String, reason: String },
    #[error("读取响应失败，目标: {host}，原因: {reason}")]
    ResponseRead { host: String, reason: String },
    #[error("响应体超过 {limit} 字节上限，目标: {host}")]
    ResponseTooLarge { host: String, limit: usize },
    #[error("解析响应文本失败，目标: {host}，原因: {reason}")]
    ResponseText { host: String, reason: String },
}

/// 一次在 Tokio 控制面执行的请求；接收端可在 GPUI 执行器上等待。
pub struct ApiExchange {
    receiver: oneshot::Receiver<Result<ApiResponseSnapshot, ApiError>>,
    task: JoinHandle<()>,
}

/// 全局唯一的 API HTTP 客户端服务。
#[derive(Clone)]
pub struct ApiHttpService {
    client: Client,
    runtime: Handle,
}

impl ApiHttpService {
    /// 构造长生命周期 HTTP 客户端；TLS/HTTP2 初始化失败时返回明确错误。
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

    /// 在 Tokio 控制面发起请求，返回可由 UI 等待的一次性结果。
    pub fn start(
        &self,
        request: ApiRequest,
        environment: ApiEnvironment,
        cookies: Vec<ApiCookie>,
    ) -> ApiExchange {
        let client = self.client.clone();
        let (sender, receiver) = oneshot::channel();
        let task = self.runtime.spawn(async move {
            let result = execute_request(&client, &request, &environment, &cookies).await;
            let _ = sender.send(result);
        });
        ApiExchange { receiver, task }
    }

    /// 直接执行请求；测试和 Tokio 侧调用使用。
    ///
    /// # Errors
    /// URL、认证、网络或响应读取失败时返回 [`ApiError`]。
    pub async fn execute(
        &self,
        request: &ApiRequest,
        environment: &ApiEnvironment,
        cookies: &[ApiCookie],
    ) -> Result<ApiResponseSnapshot, ApiError> {
        execute_request(&self.client, request, environment, cookies).await
    }
}

impl ApiExchange {
    /// 等待 Tokio 控制面返回结果。
    ///
    /// # Errors
    /// 请求构造、传输、响应读取失败，或执行任务在返回前中止时返回
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
    environment: &ApiEnvironment,
    cookies: &[ApiCookie],
) -> Result<ApiResponseSnapshot, ApiError> {
    match request.protocol {
        ApiProtocol::Http => send_http(client, request, environment, cookies, false).await,
        ApiProtocol::GraphQl => send_http(client, request, environment, cookies, true).await,
        ApiProtocol::Grpc => send_grpc(client, request, environment).await,
        ApiProtocol::WebSocket => Err(ApiError::UnsupportedProtocol {
            protocol: ApiProtocol::WebSocket.as_str().to_string(),
        }),
    }
}

async fn send_http(
    client: &Client,
    request: &ApiRequest,
    environment: &ApiEnvironment,
    cookies: &[ApiCookie],
    graphql: bool,
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
    let method = if graphql {
        Method::POST
    } else {
        method_for(request.method)
    };
    let PreparedRequestParts {
        mut headers,
        mut body,
    } = prepared_http_parts(request, environment, graphql)?;
    apply_cookies(&url, cookies, &mut headers);
    apply_auth(AuthRequest {
        client,
        request,
        environment,
        headers: &mut headers,
        body: &body,
        url: &url,
        method: method.as_str(),
    })
    .await?;
    let mut builder = client
        .request(method, url.clone())
        .body(std::mem::take(&mut body));
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
    request: &ApiRequest,
    environment: &ApiEnvironment,
) -> Result<ApiResponseSnapshot, ApiError> {
    if request.grpc_service.trim().is_empty() || request.grpc_method.trim().is_empty() {
        return Err(ApiError::InvalidAuth {
            reason: "gRPC 服务名和方法名不能为空".to_string(),
        });
    }
    let base = environment.resolve(request.url.trim());
    let url = Url::parse(&base).map_err(|error| ApiError::InvalidUrl {
        host: safe_url_label(&base),
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
        headers.push((
            environment.resolve(&header.name),
            environment.resolve(&header.value),
        ));
    }
    apply_auth(AuthRequest {
        client,
        request,
        environment,
        headers: &mut headers,
        body: &framed,
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
    let mut snapshot = read_response(response, started, &host).await?;
    if let Some(payload) = grpc_payload(snapshot.body.as_bytes()) {
        snapshot.body = STANDARD.encode(payload);
    } else {
        snapshot.body = STANDARD.encode(snapshot.body.as_bytes());
    }
    Ok(snapshot)
}

fn prepared_http_parts(
    request: &ApiRequest,
    environment: &ApiEnvironment,
    graphql: bool,
) -> Result<PreparedRequestParts, ApiError> {
    let mut headers = request
        .headers
        .iter()
        .filter(|header| header.enabled)
        .map(|header| {
            (
                environment.resolve(&header.name),
                environment.resolve(&header.value),
            )
        })
        .collect::<Vec<_>>();
    if graphql {
        ensure_header(&mut headers, "content-type", "application/json");
        let variables: serde_json::Value =
            serde_json::from_str(environment.resolve(&request.graphql_variables).as_str())
                .map_err(|error| ApiError::InvalidGraphQlVariables {
                    reason: error.to_string(),
                })?;
        let body = serde_json::json!({
            "query": environment.resolve(&request.body),
            "variables": variables
        });
        return Ok(PreparedRequestParts {
            headers,
            body: serde_json::to_vec(&body).map_err(|error| ApiError::InvalidGraphQlVariables {
                reason: error.to_string(),
            })?,
        });
    }
    Ok(PreparedRequestParts {
        headers,
        body: request.body.as_bytes().to_vec(),
    })
}

async fn apply_auth(input: AuthRequest<'_>) -> Result<(), ApiError> {
    let AuthRequest {
        client,
        request,
        environment,
        headers,
        body,
        url,
        method,
    } = input;
    match &request.auth {
        ApiAuth::None => Ok(()),
        ApiAuth::Bearer { token_env } => {
            let token = env_value(environment, token_env)?;
            ensure_header(headers, "authorization", &format!("Bearer {token}"));
            Ok(())
        }
        ApiAuth::Basic {
            username_env,
            password_env,
        } => {
            let username = env_value(environment, username_env)?;
            let password = env_value(environment, password_env)?;
            let token = STANDARD.encode(format!("{username}:{password}"));
            ensure_header(headers, "authorization", &format!("Basic {token}"));
            Ok(())
        }
        ApiAuth::OAuth2ClientCredentials {
            token_url,
            client_id,
            client_secret_env,
            scope,
        } => {
            let token_url = environment.resolve(token_url);
            let secret = env_value(environment, client_secret_env)?;
            let mut form = vec![
                ("grant_type", "client_credentials"),
                ("client_id", client_id.as_str()),
                ("client_secret", secret.as_str()),
            ];
            if !scope.is_empty() {
                form.push(("scope", scope.as_str()));
            }
            let response = client
                .post(&token_url)
                .form(&form)
                .send()
                .await
                .map_err(|error| ApiError::OAuthToken {
                    url: safe_url_label(&token_url),
                    reason: error.to_string(),
                })?;
            let value: serde_json::Value =
                response
                    .json()
                    .await
                    .map_err(|error| ApiError::OAuthToken {
                        url: safe_url_label(&token_url),
                        reason: error.to_string(),
                    })?;
            let token = value
                .get("access_token")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ApiError::OAuthToken {
                    url: safe_url_label(&token_url),
                    reason: "响应缺少 access_token".to_string(),
                })?;
            ensure_header(headers, "authorization", &format!("Bearer {token}"));
            Ok(())
        }
        ApiAuth::AwsSigV4 {
            access_key_env,
            secret_key_env,
            session_token_env,
            region,
            service,
        } => {
            let access_key = env_value(environment, access_key_env)?;
            let secret_key = env_value(environment, secret_key_env)?;
            let session_token = if session_token_env.is_empty() {
                None
            } else {
                Some(env_value(environment, session_token_env)?)
            };
            super::signing::sign_aws_v4(
                method,
                url,
                headers,
                body,
                &super::signing::AwsSigV4Params {
                    access_key: &access_key,
                    secret_key: &secret_key,
                    session_token: session_token.as_deref(),
                    region,
                    service,
                    now: Utc::now(),
                },
            )
        }
    }
}

fn env_value(environment: &ApiEnvironment, name: &str) -> Result<String, ApiError> {
    environment
        .variables
        .iter()
        .find(|variable| variable.enabled && variable.name == name)
        .map(|variable| variable.value.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::InvalidAuth {
            reason: format!("环境变量 {name} 未配置或为空"),
        })
}

fn apply_cookies(url: &Url, cookies: &[ApiCookie], headers: &mut Vec<(String, String)>) {
    let values = cookies
        .iter()
        .filter(|cookie| cookie.applies_to(url))
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>();
    if !values.is_empty() {
        let value = values.join("; ");
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
            body: String::new(),
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
            .execute(&request, &ApiEnvironment::empty(), &[cookie])
            .await
            .expect("请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 200);
        assert!(response.body.contains("\"ok\": true"));
        assert_eq!(response.set_cookies[0].name, "theme");
        assert_eq!(response.set_cookies[0].value, "dark");
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
            .execute(&request, &environment, &[])
            .await
            .expect("GraphQL 请求成功");
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
            .execute(&request, &environment, &[])
            .await
            .expect("OAuth 请求成功");
        server.await.expect("测试服务结束");
        assert_eq!(response.status, 201);
    }
}
