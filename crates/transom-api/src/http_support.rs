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
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = read_limited_body(response, host).await?;
    let body_size = body.len();
    let body = display_body(body, content_type.as_deref());
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
        let (attribute, value) = part
            .split_once('=')
            .map_or((part, None), |(name, value)| (name, Some(value.trim())));
        match attribute.trim().to_ascii_lowercase().as_str() {
            "domain" => {
                if let Some(domain) = value.filter(|value| !value.is_empty()) {
                    cookie.domain = Some(domain.to_string());
                }
            }
            "path" => {
                if let Some(path) = value.filter(|value| !value.is_empty()) {
                    cookie.path = path.to_string();
                }
            }
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "max-age" => {
                cookie.expires_at =
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
                cookie.expires_at = value
                    .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
                    .map(|time| time.timestamp().max(0) as u64);
            }
            _ => {}
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

fn display_body(body: Vec<u8>, content_type: Option<&str>) -> String {
    let body_size = body.len();
    let text = match String::from_utf8(body) {
        Ok(text) => text,
        Err(error) if is_textual_content_type(content_type) => {
            String::from_utf8_lossy(&error.into_bytes()).into_owned()
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

pub(super) fn grpc_payload(body: &[u8]) -> Option<&[u8]> {
    if body.len() < 5 || body[0] != 0 {
        return None;
    }
    let length = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    body.get(5..5usize.checked_add(length)?)
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
        )
        .expect("Cookie 应可解析");
        assert_eq!(cookie.domain.as_deref(), Some("Example.COM"));
        assert_eq!(cookie.path, "/api");
        assert!(cookie.secure);
        assert!(cookie.http_only);
        assert_eq!(cookie.expires_at, Some(0));
    }

    #[test]
    fn binary_response_is_described_instead_of_becoming_an_error() {
        let body = display_body(vec![0xff, 0x00, 0x01], Some("image/png"));
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
    fn request_host_omits_path_and_query() {
        let url =
            reqwest::Url::parse("https://example.com/private/users?token=secret").expect("URL");
        assert_eq!(request_host(&url), "https://example.com");
    }
}
