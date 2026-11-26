//! 本地 Cookie 模型、匹配规则与手工文本解析。

use super::util::unix_now;
use serde::{Deserialize, Serialize};
use transom_core::AppError;

/// 本地 Cookie；请求发送时按域名、过期时间和 Secure 规则筛选。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiCookie {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default = "default_cookie_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

fn default_cookie_path() -> String {
    "/".to_string()
}

impl ApiCookie {
    /// 是否适用于当前请求地址。
    pub fn applies_to(&self, url: &reqwest::Url) -> bool {
        let host = url.host_str().unwrap_or_default();
        let domain = self.domain.as_deref().unwrap_or(host);
        cookie_domain_matches(host, domain)
            && cookie_path_matches(url.path(), &self.path)
            && (!self.secure || url.scheme() == "https")
            && self.expires_at.is_none_or(|expires| expires > unix_now())
    }
}

pub(crate) fn optional_domain_eq(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left
            .trim_start_matches('.')
            .eq_ignore_ascii_case(right.trim_start_matches('.')),
        _ => false,
    }
}

pub(crate) fn cookie_domain_matches(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = domain
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn cookie_path_matches(request_path: &str, cookie_path: &str) -> bool {
    let cookie_path = if cookie_path.trim().is_empty() {
        "/"
    } else {
        cookie_path.trim()
    };
    if cookie_path == "/" || request_path == cookie_path {
        return true;
    }
    request_path.starts_with(cookie_path)
        && request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
}

pub(crate) fn is_valid_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii()
                && !byte.is_ascii_control()
                && !matches!(
                    byte,
                    b' ' | b'('
                        | b')'
                        | b'<'
                        | b'>'
                        | b'@'
                        | b','
                        | b';'
                        | b':'
                        | b'\\'
                        | b'"'
                        | b'/'
                        | b'['
                        | b']'
                        | b'?'
                        | b'='
                        | b'{'
                        | b'}'
                )
        })
}

pub(crate) fn is_valid_cookie_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| !byte.is_ascii_control() && byte != b';')
}

pub(crate) fn cookie_header_value(url: &reqwest::Url, cookies: &[ApiCookie]) -> Option<String> {
    let values = cookies
        .iter()
        .filter(|cookie| cookie.applies_to(url))
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        Some(values.join("; "))
    }
}

/// 将项目 Cookie 渲染为 `name=value` 文本；域名和属性由响应自动维护。
pub fn cookies_text(cookies: &[ApiCookie]) -> String {
    cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 `name=value` 文本替换 Cookie；域名缺省时按发送目标主机生效。
///
/// # Errors
/// 非空行没有等号、Cookie 名称非法或值包含控制字符时返回中文错误。
pub fn parse_cookies_text(text: &str) -> Result<Vec<ApiCookie>, AppError> {
    let mut cookies = Vec::with_capacity(text.lines().count());
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("Cookie 第 {} 行缺少等号: {}", index + 1, trimmed),
            ));
        };
        let name = name.trim();
        if !is_valid_cookie_name(name) {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("Cookie 第 {} 行名称非法", index + 1),
            ));
        }
        let value = value.trim();
        if !is_valid_cookie_value(value) {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("Cookie 第 {} 行值包含非法字符", index + 1),
            ));
        }
        cookies.push(ApiCookie {
            name: name.to_string(),
            value: value.to_string(),
            domain: None,
            path: "/".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        });
    }
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_cookie_text_rejects_invalid_name_and_control_characters() {
        assert!(parse_cookies_text("bad name=value").is_err());
        assert!(parse_cookies_text("name=value\r\nX-Evil: 1").is_err());
    }

    #[test]
    fn cookie_domain_requires_exact_or_subdomain_match() {
        assert!(cookie_domain_matches("api.example.com", ".example.com"));
        assert!(!cookie_domain_matches("evil.example", "example.com"));
    }
}
