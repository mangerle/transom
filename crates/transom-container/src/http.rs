//! 任意双向流上的 HTTP/1.1 客户端。
//!
//! 设计原理（Why）：
//! - 容器守护进程的 HTTP 协议与宿主通道解耦，hyper 在 [`EngineStream`]
//!   上完成握手后，Unix socket、命名管道、TCP 与子进程 stdio 共用同一套
//!   请求/响应代码；
//! - `exec` 必须拿到原始双向流，因此 Upgrade 路径单独实现：连接驱动任务
//!   必须持续运行到升级完成，不能在响应返回时被提前回收；
//! - 普通请求的驱动任务挂在响应守卫上：响应体读完或调用方放弃时立即中止，
//!   不留下无人回收的后台任务。

use crate::bridge::BridgeItem;
use crate::error::ContainerError;
use crate::transport::{EngineStream, EngineTransport};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use std::time::Duration;

/// 单次请求的总超时；日志跟随与 exec 流不套用此上限。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 响应体读取总超时：守护进程或远端代理半死不活时不能永久占住任务。
const BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// 错误响应体最多保留的字节数，避免异常响应打满日志。
const ERROR_BODY_LIMIT: usize = 8 * 1024;

/// 请求体类型。
pub type RequestBody = Full<Bytes>;

/// 未消费的 HTTP 响应。
pub struct RawResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Incoming,
    /// 连接驱动任务守卫：响应释放时中止后台驱动。
    _connection: ConnectionGuard,
}

struct ConnectionGuard(Option<tokio::task::JoinHandle<hyper::Result<()>>>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

/// 发起一次普通 HTTP 请求，返回未消费的响应。
///
/// # Errors
/// 通道建立失败、HTTP 握手失败或请求发送失败时返回
/// [`ContainerError::Http`] / [`ContainerError::Connect`]。
pub async fn request(
    transport: &EngineTransport,
    method: Method,
    path: &str,
    body: Bytes,
    content_type: Option<&str>,
) -> Result<RawResponse, ContainerError> {
    request_with_headers(transport, method, path, body, content_type, &[]).await
}

/// 支持携带额外请求头的 HTTP 请求（如 Registry 认证头 X-Registry-Auth）。
pub async fn request_with_headers(
    transport: &EngineTransport,
    method: Method,
    path: &str,
    body: Bytes,
    content_type: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> Result<RawResponse, ContainerError> {
    request_with_timeout(
        transport,
        method,
        path,
        body,
        content_type,
        extra_headers,
        REQUEST_TIMEOUT,
    )
    .await
}

/// 支持自定义超时与额外请求头的 HTTP 请求（用于镜像构建等长耗时操作）。
pub async fn request_with_timeout(
    transport: &EngineTransport,
    method: Method,
    path: &str,
    body: Bytes,
    content_type: Option<&str>,
    extra_headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<RawResponse, ContainerError> {
    let operation = format!("{method} {path}");
    let stream = transport.connect().await?;
    let (mut sender, connection) = tokio::time::timeout(
        REQUEST_TIMEOUT,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .map_err(|_| ContainerError::Http {
        operation: operation.clone(),
        message: "HTTP 握手超时".to_string(),
    })?
    .map_err(|error| ContainerError::Http {
        operation: operation.clone(),
        message: error.to_string(),
    })?;
    let driver = tokio::spawn(connection);
    let mut request = build_request(method, path, body, content_type, false)?;
    for (name, value) in extra_headers {
        if let (Ok(h_name), Ok(h_val)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            hyper::header::HeaderValue::from_str(value),
        ) {
            request.headers_mut().insert(h_name, h_val);
        }
    }
    let response = tokio::time::timeout(timeout, sender.send_request(request))
        .await
        .map_err(|_| ContainerError::Http {
            operation: operation.clone(),
            message: "请求超时".to_string(),
        })?
        .map_err(|error| ContainerError::Http {
            operation,
            message: error.to_string(),
        })?;
    let (parts, body) = response.into_parts();
    Ok(RawResponse {
        status: parts.status,
        headers: parts.headers,
        body,
        _connection: ConnectionGuard(Some(driver)),
    })
}

/// 把响应体泵进阻塞侧通道，并返回泵任务。
///
/// 设计原理（Why）：连接驱动任务的生命周期挂在响应守卫上，而归档与镜像进度这类
/// 响应要边收边解析、可能持续数分钟。若调用方只把 `response.body` 搬进自己的任务，
/// `RawResponse` 会在函数返回时析构并中止驱动任务，真机表现为读取中途报
/// 「connection closed before message completed」，且因为取决于服务端发完数据的快慢
/// 而呈偶发。这里要求整体接管响应，守卫与响应体同生同灭。
///
/// 代价：调用方不再能读状态码与响应头，只适用于「已经判定 2xx」之后的流式读取。
pub fn pump_body(
    response: RawResponse,
    sender: flume::Sender<BridgeItem>,
) -> tokio::task::JoinHandle<()> {
    let RawResponse {
        status: _,
        headers: _,
        body,
        _connection,
    } = response;
    // 守卫留在任务内部：解析结束或任务被中止时才释放连接。
    tokio::spawn(async move {
        let _connection = _connection;
        let mut body = std::pin::pin!(body);
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => {
                    let Ok(data) = frame.into_data() else {
                        continue;
                    };
                    if sender.send_async(Ok(data.to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send_async(Err(error.to_string())).await;
                    break;
                }
            }
        }
    })
}

/// 发起一次带 Upgrade 的请求，返回升级后的原始双向流。
///
/// 用于 `POST /exec/{id}/start` 与 attach：服务端确认升级后，后续字节
/// 不再属于 HTTP 报文。
///
/// # Errors
/// 握手失败、服务端拒绝升级或升级过程失败时返回 [`ContainerError::Http`]；
/// 非 101 响应会附带响应体摘要，便于定位权限/参数问题。
pub async fn request_upgrade(
    transport: &EngineTransport,
    method: Method,
    path: &str,
    body: Bytes,
    content_type: Option<&str>,
) -> Result<EngineStream, ContainerError> {
    let operation = format!("{method} {path}（升级）");
    let stream = transport.connect().await?;
    let (mut sender, connection) = tokio::time::timeout(
        REQUEST_TIMEOUT,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .map_err(|_| ContainerError::Http {
        operation: operation.clone(),
        message: "HTTP 握手超时".to_string(),
    })?
    .map_err(|error| ContainerError::Http {
        operation: operation.clone(),
        message: error.to_string(),
    })?;
    let connection_task = tokio::spawn(async move { connection.with_upgrades().await });
    let request = build_request(method, path, body, content_type, true)?;
    let mut response = tokio::time::timeout(REQUEST_TIMEOUT, sender.send_request(request))
        .await
        .map_err(|_| ContainerError::Http {
            operation: operation.clone(),
            message: "发送升级请求超时".to_string(),
        })?
        .map_err(|error| ContainerError::Http {
            operation: operation.clone(),
            message: error.to_string(),
        })?;
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        let status = response.status();
        let body = read_body_text(response.into_body(), ERROR_BODY_LIMIT).await;
        connection_task.abort();
        return Err(if status == StatusCode::NOT_FOUND {
            ContainerError::NotFound {
                target: path.to_string(),
            }
        } else {
            ContainerError::Api {
                operation,
                status: status.as_u16(),
                body,
            }
        });
    }
    let upgraded = tokio::time::timeout(REQUEST_TIMEOUT, hyper::upgrade::on(&mut response))
        .await
        .map_err(|_| ContainerError::Http {
            operation: operation.clone(),
            message: "等待连接升级超时".to_string(),
        })?
        .map_err(|error| ContainerError::Http {
            operation: operation.clone(),
            message: format!("升级连接失败: {error}"),
        })?;
    // 升级完成后连接驱动的使命结束；等待它以释放任务句柄。
    let _ = connection_task.await;
    Ok(Box::new(TokioIo::new(upgraded)))
}

fn build_request(
    method: Method,
    path: &str,
    body: Bytes,
    content_type: Option<&str>,
    upgrade: bool,
) -> Result<Request<RequestBody>, ContainerError> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(hyper::header::HOST, "localhost");
    if let Some(content_type) = content_type {
        builder = builder.header(hyper::header::CONTENT_TYPE, content_type);
    }
    if upgrade {
        builder = builder
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::UPGRADE, "tcp");
    }
    builder
        .body(Full::new(body))
        .map_err(|error| ContainerError::Http {
            operation: format!("构造请求 {path}"),
            message: error.to_string(),
        })
}

/// 发起一次流式请求体的 HTTP 请求。
///
/// 用于容器归档上传：请求体由后台 tar 生成器分块产出，客户端不等整个
/// 文件读入内存；没有精确长度时 hyper 自动使用 chunked 编码。
///
/// # Errors
/// 通道建立、握手或发送失败时返回 [`ContainerError`]。
pub async fn request_stream<B>(
    transport: &EngineTransport,
    method: Method,
    path: &str,
    body: B,
    content_type: Option<&str>,
) -> Result<RawResponse, ContainerError>
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let operation = format!("{method} {path}");
    let stream = transport.connect().await?;
    let (mut sender, connection) = tokio::time::timeout(
        REQUEST_TIMEOUT,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .map_err(|_| ContainerError::Http {
        operation: operation.clone(),
        message: "HTTP 握手超时".to_string(),
    })?
    .map_err(|error| ContainerError::Http {
        operation: operation.clone(),
        message: error.to_string(),
    })?;
    let driver = tokio::spawn(connection);
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(hyper::header::HOST, "localhost");
    if let Some(content_type) = content_type {
        builder = builder.header(hyper::header::CONTENT_TYPE, content_type);
    }
    let request = builder.body(body).map_err(|error| ContainerError::Http {
        operation: operation.clone(),
        message: error.to_string(),
    })?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|error| ContainerError::Http {
            operation,
            message: error.to_string(),
        })?;
    let (parts, body) = response.into_parts();
    Ok(RawResponse {
        status: parts.status,
        headers: parts.headers,
        body,
        _connection: ConnectionGuard(Some(driver)),
    })
}

/// 读取响应体前缀，最多保留 `limit` 字节，并返回是否还有被丢弃的数据。
///
/// 设计原理（Why）：必须先按帧累计再判断上限，不能 `collect()` 后才知道大小；
/// 否则远端引擎返回超大响应时会先整段读入内存。达到上限后停止读取，由调用方
/// 根据语义决定是截断展示还是报错；连接守卫会在响应释放时终止驱动。
async fn read_body_prefix<B>(body: B, limit: usize) -> Result<(Vec<u8>, bool), ContainerError>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    let deadline = tokio::time::Instant::now() + BODY_READ_TIMEOUT;
    let mut body = std::pin::pin!(body);
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut truncated = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(ContainerError::Decode {
                operation: "读取响应体".to_string(),
                message: "读取响应体超时".to_string(),
            });
        }
        let frame = match tokio::time::timeout(remaining, body.frame()).await {
            Err(_) => {
                return Err(ContainerError::Decode {
                    operation: "读取响应体".to_string(),
                    message: "读取响应体超时".to_string(),
                });
            }
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                return Err(ContainerError::Decode {
                    operation: "读取响应体".to_string(),
                    message: error.to_string(),
                });
            }
            Ok(Some(Ok(frame))) => frame,
        };
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let capacity = limit.saturating_sub(bytes.len());
        if data.len() > capacity {
            bytes.extend_from_slice(&data[..capacity]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&data);
    }
    Ok((bytes, truncated))
}

/// 读取响应体的文本摘要（错误诊断用），超限截断。
pub async fn read_body_text<B>(body: B, limit: usize) -> String
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    match read_body_prefix(body, limit).await {
        Ok((bytes, _)) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(error) => format!("读取响应体失败: {error}"),
    }
}

/// 读取完整响应体（用于小体积 JSON 响应）。
///
/// # Errors
/// 响应体读取失败、超时或超过上限时返回 [`ContainerError::Decode`]。
pub async fn read_body_bytes<B>(body: B, limit: usize) -> Result<Bytes, ContainerError>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    let (bytes, truncated) = read_body_prefix(body, limit).await?;
    if truncated {
        return Err(ContainerError::Decode {
            operation: "读取响应体".to_string(),
            message: format!("响应超过上限 {limit} 字节"),
        });
    }
    Ok(Bytes::from(bytes))
}

/// 把引擎返回的非成功状态码转成错误；`operation` 用于错误上下文。
pub fn ensure_success(
    status: StatusCode,
    operation: &str,
    body: String,
    target: Option<&str>,
) -> Result<(), ContainerError> {
    if status.is_success() {
        return Ok(());
    }
    Err(status_error(
        status,
        operation,
        body,
        target.unwrap_or(operation),
    ))
}

/// 构造状态码错误；404 单独映射为“目标不存在”，方便界面给出可操作提示。
pub fn status_error(
    status: StatusCode,
    operation: &str,
    body: String,
    target: &str,
) -> ContainerError {
    if status == StatusCode::NOT_FOUND {
        return ContainerError::NotFound {
            target: target.to_string(),
        };
    }
    ContainerError::Api {
        operation: operation.to_string(),
        status: status.as_u16(),
        body,
    }
}

/// 百分比编码查询参数值；只保留 RFC 3986 的非保留字符。
pub fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// 响应头里的单个值转成字符串。
pub fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value: &HeaderValue| value.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{percent_encode, read_body_bytes, read_body_prefix, read_body_text};
    use bytes::Bytes;
    use http_body_util::Full;

    #[test]
    fn percent_encoding_keeps_unreserved_and_encodes_separators() {
        assert_eq!(percent_encode("/var/lib"), "%2Fvar%2Flib");
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(percent_encode("a b"), "a%20b");
    }

    #[tokio::test]
    async fn body_prefix_stops_at_limit_and_marks_truncation() {
        let body = Full::new(Bytes::from(vec![b'a'; 64]));
        let (prefix, truncated) = read_body_prefix(body, 16).await.expect("读取前缀必须成功");
        assert_eq!(prefix.len(), 16);
        assert!(truncated);
    }

    #[tokio::test]
    async fn body_reads_reject_oversized_json_but_accept_exact_limit() {
        let oversized = Full::new(Bytes::from(vec![b'a'; 32]));
        let error = read_body_bytes(oversized, 16)
            .await
            .expect_err("超过上限必须报错");
        assert!(matches!(error, crate::error::ContainerError::Decode { .. }));

        let exact = Full::new(Bytes::from(vec![b'a'; 16]));
        let bytes = read_body_bytes(exact, 16)
            .await
            .expect("恰好达到上限必须允许");
        assert_eq!(bytes.len(), 16);

        let textual = Full::new(Bytes::from(vec![b'b'; 32]));
        let text = read_body_text(textual, 8).await;
        assert_eq!(text, "bbbbbbbb");
    }
}
