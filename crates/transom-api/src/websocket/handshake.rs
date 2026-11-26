//! WebSocket 握手请求、Cookie 与认证头构建。

use super::ApiWebSocketError;
use crate::http_support::safe_url_label;
use crate::model::cookie_header_value;
use crate::{ApiAuth, ApiCookie, ApiEnvironment, ApiRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue, Request};

pub(super) fn build_handshake(
    request: &ApiRequest,
    environment: &ApiEnvironment,
    cookies: &[ApiCookie],
) -> Result<Request<()>, ApiWebSocketError> {
    let url =
        reqwest::Url::parse(&request.url).map_err(|error| ApiWebSocketError::BuildRequest {
            host: safe_url_label(&request.url),
            reason: error.to_string(),
        })?;
    let mut handshake = request.url.clone().into_client_request().map_err(|error| {
        ApiWebSocketError::BuildRequest {
            host: safe_url_label(&request.url),
            reason: error.to_string(),
        }
    })?;
    for header in request.headers.iter().filter(|header| header.enabled) {
        insert_header(&mut handshake, &header.name, &header.value)?;
    }
    if let Some(value) = cookie_header_value(&url, cookies) {
        append_cookie_header(&mut handshake, &value)?;
    }
    apply_websocket_auth(&mut handshake, environment, &request.auth)?;
    Ok(handshake)
}

fn insert_header(
    request: &mut Request<()>,
    name: &str,
    value: &str,
) -> Result<(), ApiWebSocketError> {
    let header_name: HeaderName =
        name.parse::<HeaderName>()
            .map_err(|error| ApiWebSocketError::InvalidHeaderName {
                name: name.to_string(),
                reason: error.to_string(),
            })?;
    let value =
        HeaderValue::from_str(value).map_err(|error| ApiWebSocketError::InvalidHeaderValue {
            name: name.to_string(),
            reason: error.to_string(),
        })?;
    request.headers_mut().insert(header_name, value);
    Ok(())
}

fn append_cookie_header(request: &mut Request<()>, cookie: &str) -> Result<(), ApiWebSocketError> {
    let existing = request
        .headers()
        .get("cookie")
        .map(HeaderValue::to_str)
        .transpose()
        .map_err(|error| ApiWebSocketError::InvalidHeaderValue {
            name: "cookie".to_string(),
            reason: error.to_string(),
        })?;
    let value = match existing.filter(|value| !value.is_empty()) {
        Some(existing) => format!("{existing}; {cookie}"),
        None => cookie.to_string(),
    };
    insert_header(request, "cookie", &value)
}

fn apply_websocket_auth(
    request: &mut Request<()>,
    environment: &ApiEnvironment,
    auth: &ApiAuth,
) -> Result<(), ApiWebSocketError> {
    let value = match auth {
        ApiAuth::None => return Ok(()),
        ApiAuth::Bearer { token_env } => {
            let token = environment_value(environment, token_env)?;
            format!("Bearer {token}")
        }
        ApiAuth::Basic {
            username_env,
            password_env,
        } => {
            let username = environment_value(environment, username_env)?;
            let password = environment_value(environment, password_env)?;
            format!(
                "Basic {}",
                STANDARD.encode(format!("{username}:{password}"))
            )
        }
        ApiAuth::OAuth2ClientCredentials { .. } | ApiAuth::AwsSigV4 { .. } => {
            let kind = match auth {
                ApiAuth::OAuth2ClientCredentials { .. } => "OAuth 2.0",
                ApiAuth::AwsSigV4 { .. } => "AWS SigV4",
                _ => "未知",
            };
            return Err(ApiWebSocketError::UnsupportedAuth {
                kind: kind.to_string(),
            });
        }
    };
    let value = HeaderValue::from_str(&value).map_err(|error| ApiWebSocketError::InvalidAuth {
        reason: format!("认证头内容无效: {error}"),
    })?;
    request.headers_mut().insert("authorization", value);
    Ok(())
}

fn environment_value(
    environment: &ApiEnvironment,
    name: &str,
) -> Result<String, ApiWebSocketError> {
    environment
        .variables
        .iter()
        .find(|variable| variable.enabled && variable.name == name)
        .map(|variable| variable.value.clone())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiWebSocketError::InvalidAuth {
            reason: format!("认证环境变量 {name} 未配置或为空"),
        })
}
