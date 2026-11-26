//! HTTP 响应读取、Cookie 解析与请求头转换辅助函数。

use super::{ApiCookie, ApiError, ApiResponseSnapshot};
use crate::model::{cookie_domain_matches, is_valid_cookie_name, is_valid_cookie_value};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Method, Response, Url};
use std::time::Instant;

const RESPONSE_FORMAT_BLOCKING_THRESHOLD: usize = 64 * 1024;

struct RawResponseSnapshot {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    elapsed_millis: u128,
    set_cookies: Vec<ApiCookie>,
    content_type: Option<String>,
}

impl RawResponseSnapshot {
    fn into_snapshot(self, body: String, body_size: usize) -> ApiResponseSnapshot {
        ApiResponseSnapshot {
            status: self.status,
            status_text: self.status_text,
            headers: self.headers,
            body_size,
            body,
            elapsed_millis: self.elapsed_millis,
            set_cookies: self.set_cookies,
        }
    }
}

pub(super) async fn read_response(
    response: Response,
    started: Instant,
    host: &str,
) -> Result<ApiResponseSnapshot, ApiError> {
    let mut raw = read_raw_response(response, started, host).await?;
    let body_size = raw.body.len();
    let content_type = raw.content_type.take();
    let body_bytes = std::mem::take(&mut raw.body);
    let body = format_response_body(body_bytes, content_type, host).await?;
    Ok(raw.into_snapshot(body, body_size))
}

pub(super) async fn read_grpc_response(
    response: Response,
    started: Instant,
    host: &str,
) -> Result<ApiResponseSnapshot, ApiError> {
    let mut raw = read_raw_response(response, started, host).await?;
    let body_size = raw.body.len();
    let body_bytes = std::mem::take(&mut raw.body);
    let body = grpc_display_body(&body_bytes);
    Ok(raw.into_snapshot(body, body_size))
}

async fn format_response_body(
    body: Vec<u8>,
    content_type: Option<String>,
    host: &str,
) -> Result<String, ApiError> {
    if body.len() < RESPONSE_FORMAT_BLOCKING_THRESHOLD {
        return Ok(display_body(&body, content_type.as_deref()));
    }
    let host = host.to_string();
    tokio::task::spawn_blocking(move || display_body(&body, content_type.as_deref()))
        .await
        .map_err(|error| ApiError::ResponseRead {
            host,
            reason: format!("响应体格式化任务异常结束: {error}"),
        })
}

async fn read_raw_response(
    response: Response,
    started: Instant,
    host: &str,
) -> Result<RawResponseSnapshot, ApiError> {
    let status = response.status();
    let response_host = response.url().host_str().map(str::to_ascii_lowercase);
    let response_path = response.url().path().to_string();
    let mut set_cookies = Vec::new();
    for value in response.headers().get_all("set-cookie") {
        if let Ok(text) = value.to_str()
            && let Some(cookie) = parse_set_cookie(text, response_host.as_deref(), &response_path)
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
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = read_limited_body(response, host).await?;
    Ok(RawResponseSnapshot {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
        elapsed_millis: started.elapsed().as_millis(),
        set_cookies,
        content_type,
    })
}

pub(super) fn parse_set_cookie(
    text: &str,
    response_host: Option<&str>,
    response_path: &str,
) -> Option<ApiCookie> {
    let mut parts = text.split(';');
    let first = parts.next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if !is_valid_cookie_name(name) || !is_valid_cookie_value(value) {
        return None;
    }
    let mut cookie = ApiCookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: response_host.map(str::to_string),
        path: default_cookie_path(response_path),
        secure: false,
        http_only: false,
        expires_at: None,
    };
    let mut max_age_expires_at = None;
    let mut expires_at = None;
    for part in parts {
        let part = part.trim();
        let (attribute, value) = part
            .split_once('=')
            .map_or((part, None), |(name, value)| (name, Some(value.trim())));
        match attribute.trim().to_ascii_lowercase().as_str() {
            "domain" => {
                let domain = value.filter(|value| !value.is_empty())?;
                let host = response_host?;
                if !cookie_domain_matches(host, domain) {
                    return None;
                }
                cookie.domain = Some(domain.to_string());
            }
            "path" => {
                if let Some(path) = value.filter(|path| path.starts_with('/')) {
                    cookie.path = path.to_string();
                }
            }
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "max-age" => {
                max_age_expires_at =
                    value
                        .and_then(|value| value.parse::<i64>().ok())
                        .map(|seconds| {
                            if seconds <= 0 {
                                0
                            } else {
                                unix_now().saturating_add(seconds as u64)
                            }
                        });
            }
            "expires" => {
                expires_at = value
                    .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
                    .map(|time| time.timestamp().max(0) as u64);
            }
            _ => {}
        }
    }
    cookie.expires_at = max_age_expires_at.or(expires_at);
    Some(cookie)
}

fn default_cookie_path(response_path: &str) -> String {
    if !response_path.starts_with('/') || response_path.matches('/').count() <= 1 {
        return "/".to_string();
    }
    response_path
        .rfind('/')
        .filter(|index| *index > 0)
        .map_or_else(
            || "/".to_string(),
            |index| response_path[..index].to_string(),
        )
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

pub(super) fn ensure_header_if_absent(
    headers: &mut Vec<(String, String)>,
    name: &str,
    value: &str,
) {
    if !headers
        .iter()
        .any(|(header, _)| header.eq_ignore_ascii_case(name))
    {
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

pub(super) async fn read_limited_body(response: Response, host: &str) -> Result<Vec<u8>, ApiError> {
    if response
        .content_length()
        .is_some_and(|length| length > super::MAX_RESPONSE_BYTES as u64)
    {
        return Err(ApiError::ResponseTooLarge {
            host: host.to_string(),
            limit: super::MAX_RESPONSE_BYTES,
        });
    }
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

fn display_body(body: &[u8], content_type: Option<&str>) -> String {
    let body_size = body.len();
    let text = match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) if is_textual_content_type(content_type) => {
            String::from_utf8_lossy(body).into_owned()
        }
        Err(_) => {
            return format!(
                "[二进制响应，{body_size} 字节，Content-Type: {}]",
                content_type.unwrap_or("application/octet-stream")
            );
        }
    };
    let trimmed = text.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        serde_json::from_str::<serde_json::Value>(trimmed)
            .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.clone()))
            .unwrap_or(text)
    } else {
        text
    }
}

fn is_textual_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let content_type = content_type.to_ascii_lowercase();
    content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
        || content_type.contains("x-www-form-urlencoded")
        || content_type.contains("graphql")
}

#[cfg(test)]
pub(super) fn grpc_payload(body: &[u8]) -> Option<&[u8]> {
    grpc_payloads(body).into_iter().next()
}

fn grpc_display_body(body: &[u8]) -> String {
    let payloads = grpc_payloads(body);
    if payloads.is_empty() {
        STANDARD.encode(body)
    } else {
        payloads
            .into_iter()
            .map(|payload| STANDARD.encode(payload))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn grpc_payloads(body: &[u8]) -> Vec<&[u8]> {
    let mut payloads = Vec::new();
    let mut offset = 0_usize;
    while offset < body.len() {
        let Some(frame) = body.get(offset..) else {
            return Vec::new();
        };
        if frame.len() < 5 || frame[0] != 0 {
            return Vec::new();
        }
        let length = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        let Some(end) = offset
            .checked_add(5)
            .and_then(|start| start.checked_add(length))
        else {
            return Vec::new();
        };
        let Some(payload) = body.get(offset.saturating_add(5)..end) else {
            return Vec::new();
        };
        payloads.push(payload);
        offset = end;
    }
    payloads
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_attributes_are_case_insensitive() {
        let cookie = parse_set_cookie(
            "sid=abc; domain=Example.COM; Path=/api; secure; httponly; Max-Age=0",
            Some("example.com"),
            "/login",
        )
        .expect("Cookie 应可解析");
        assert_eq!(cookie.domain.as_deref(), Some("Example.COM"));
        assert_eq!(cookie.path, "/api");
        assert!(cookie.secure);
        assert!(cookie.http_only);
        assert_eq!(cookie.expires_at, Some(0));
    }

    #[test]
    fn set_cookie_rejects_domain_not_matching_response_host() {
        assert!(
            parse_set_cookie("sid=abc; Domain=example.com", Some("evil.example"), "/",).is_none()
        );
    }

    #[test]
    fn max_age_takes_precedence_over_expires_regardless_of_order() {
        let cookie = parse_set_cookie(
            "sid=abc; Expires=Wed, 09 Jun 2100 10:18:14 GMT; Max-Age=0",
            Some("example.com"),
            "/",
        )
        .expect("Cookie 应可解析");
        assert_eq!(cookie.expires_at, Some(0));
    }

    #[test]
    fn set_cookie_rejects_control_characters() {
        assert!(parse_set_cookie("sid=abc\r\nX-Evil: 1", Some("example.com"), "/").is_none());
    }

    #[test]
    fn set_cookie_default_path_uses_response_directory() {
        let cookie = parse_set_cookie("sid=abc", Some("example.com"), "/api/users")
            .expect("Cookie 应可解析");
        assert_eq!(cookie.path, "/api");
        let cookie = parse_set_cookie("sid=abc; Path=relative", Some("example.com"), "/api/users")
            .expect("Cookie 应可解析");
        assert_eq!(cookie.path, "/api");
    }

    #[test]
    fn binary_response_is_described_instead_of_becoming_an_error() {
        let body = display_body(&[0xff, 0x00, 0x01], Some("image/png"));
        assert!(body.contains("二进制响应"));
        assert!(body.contains("image/png"));
    }

    #[test]
    fn grpc_payload_rejects_truncated_frames() {
        let mut body = vec![0_u8; 5];
        body[1..5].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(grpc_payload(&body).is_none());
    }

    #[test]
    fn grpc_display_preserves_non_utf8_binary_payload() {
        let body = [0, 0, 0, 0, 3, 0xff, 0x00, 0x01];
        assert_eq!(
            grpc_display_body(&body),
            STANDARD.encode([0xff, 0x00, 0x01])
        );
    }

    #[test]
    fn grpc_display_keeps_all_complete_frames() {
        let body = [0, 0, 0, 0, 1, b'a', 0, 0, 0, 0, 1, b'b'];
        assert_eq!(
            grpc_display_body(&body),
            format!("{}\n{}", STANDARD.encode(b"a"), STANDARD.encode(b"b"))
        );
    }

    #[test]
    fn request_host_omits_path_and_query() {
        let url =
            reqwest::Url::parse("https://example.com/private/users?token=secret").expect("URL");
        assert_eq!(request_host(&url), "https://example.com");
    }
}
