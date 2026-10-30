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
    let request = build_request(method, path, body, content_type, false)?;
    let response = tokio::time::timeout(REQUEST_TIMEOUT, sender.send_request(request))
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
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|error| ContainerError::Http {
            operation: operation.clone(),
            message: error.to_string(),
        })?;
    let connection_task = tokio::spawn(async move { connection.with_upgrades().await });
    let request = build_request(method, path, body, content_type, true)?;
    let mut response =
        sender
            .send_request(request)
            .await
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
    let upgraded =
        hyper::upgrade::on(&mut response)
            .await
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

/// 读取响应体的文本摘要（错误诊断用），超限截断。
pub async fn read_body_text(body: Incoming, limit: usize) -> String {
    match body.collect().await {
        Ok(collected) => {
            let bytes = collected.to_bytes();
            let slice = &bytes[..bytes.len().min(limit)];
            String::from_utf8_lossy(slice).trim().to_string()
        }
        Err(error) => format!("读取响应体失败: {error}"),
    }
}

/// 读取完整响应体（用于小体积 JSON 响应）。
///
/// # Errors
/// 响应体读取失败或超过上限时返回 [`ContainerError::Decode`]。
pub async fn read_body_bytes(body: Incoming, limit: usize) -> Result<Bytes, ContainerError> {
    let collected = body
        .collect()
        .await
        .map_err(|error| ContainerError::Decode {
            operation: "读取响应体".to_string(),
            message: error.to_string(),
        })?;
    let bytes = collected.to_bytes();
    if bytes.len() > limit {
        return Err(ContainerError::Decode {
            operation: "读取响应体".to_string(),
            message: format!("响应超过上限 {limit} 字节"),
        });
    }
    Ok(bytes)
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
    use super::percent_encode;

    #[test]
    fn percent_encoding_keeps_unreserved_and_encodes_separators() {
        assert_eq!(percent_encode("/var/lib"), "%2Fvar%2Flib");
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(percent_encode("a b"), "a%20b");
    }
}
