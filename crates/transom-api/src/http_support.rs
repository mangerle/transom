//! HTTP 响应读取、Cookie 解析与请求头转换辅助函数。

use super::{ApiCookie, ApiError, ApiResponseSnapshot};
use futures::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Method, Response, Url};
use std::time::Instant;

pub(super) async fn read_response(
    response: Response,
    started: Instant,
    host: &str,
) -> Result<ApiResponseSnapshot, ApiError> {
    let status = response.status();
    let response_host = response.url().host_str().map(str::to_ascii_lowercase);
    let mut set_cookies = Vec::new();
    for value in response.headers().get_all("set-cookie") {
        if let Ok(text) = value.to_str()
            && let Some(cookie) = parse_set_cookie(text, response_host.as_deref())
        {
            set_cookies.push(cookie);
        }
    }
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
    let body = read_limited_body(response, host).await?;
    let body_size = body.len();
    let body = display_body(body, host)?;
    Ok(ApiResponseSnapshot {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
        body_size,
        elapsed_millis: started.elapsed().as_millis(),
        set_cookies,
    })
}

pub(super) fn parse_set_cookie(text: &str, response_host: Option<&str>) -> Option<ApiCookie> {
    let mut parts = text.split(';');
    let first = parts.next()?.trim();
    let (name, value) = first.split_once('=')?;
    let mut cookie = ApiCookie {
        name: name.trim().to_string(),
        value: value.trim().to_string(),
        domain: response_host.map(str::to_string),
        path: "/".to_string(),
        secure: false,
        http_only: false,
        expires_at: None,
    };
    for part in parts {
        let part = part.trim();
        if let Some(domain) = part.strip_prefix("Domain=") {
            cookie.domain = Some(domain.to_string());
        } else if let Some(path) = part.strip_prefix("Path=") {
            cookie.path = path.to_string();
        } else if part == "Secure" {
            cookie.secure = true;
        } else if part == "HttpOnly" {
            cookie.http_only = true;
        } else if let Some(max_age) = part.strip_prefix("Max-Age=") {
            cookie.expires_at = max_age
                .parse::<u64>()
                .ok()
                .map(|seconds| unix_now().saturating_add(seconds));
        }
    }
    Some(cookie)
}

pub(super) fn ensure_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some((_, existing)) = headers
        .iter_mut()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
    {
        existing.clone_from(&value.to_string());
    } else {
        headers.push((name.to_string(), value.to_string()));
    }
}

pub(super) fn method_for(method: super::ApiMethod) -> Method {
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

pub(super) fn header_name(header: &(String, String)) -> Result<HeaderName, ApiError> {
    HeaderName::from_bytes(header.0.trim().as_bytes()).map_err(|error| ApiError::InvalidHeader {
        line: 0,
        reason: format!("名称 {} 无效: {error}", header.0),
    })
}

pub(super) fn header_value(header: &(String, String)) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(header.1.as_str()).map_err(|error| ApiError::InvalidHeaderValue {
        name: header.0.clone(),
        reason: error.to_string(),
    })
}

async fn read_limited_body(response: Response, host: &str) -> Result<Vec<u8>, ApiError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ApiError::ResponseRead {
            host: host.to_string(),
            reason: error.to_string(),
        })?;
        if body.len().saturating_add(chunk.len()) > super::MAX_RESPONSE_BYTES {
            return Err(ApiError::ResponseTooLarge {
                host: host.to_string(),
                limit: super::MAX_RESPONSE_BYTES,
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

pub(super) fn grpc_payload(body: &[u8]) -> Option<&[u8]> {
    if body.len() < 5 || body[0] != 0 {
        return None;
    }
    let length = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    body.get(5..5 + length)
}

pub(super) fn request_host(url: &Url) -> String {
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

pub(super) fn safe_url_label(input: &str) -> String {
    Url::parse(input)
        .map(|url| request_host(&url))
        .unwrap_or_else(|_| "无法解析的地址".to_string())
}

pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
