//! API 请求认证策略：Bearer、Basic、OAuth 2.0 与 AWS SigV4。

use super::super::http_support::{ensure_header, safe_url_label};
use super::super::{ApiAuth, ApiEnvironment, ApiError, ApiRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use reqwest::{Client, Url};

/// 认证阶段需要的完整请求上下文；集中传递避免参数列表继续膨胀。
pub(crate) struct AuthRequest<'a> {
    pub(crate) client: &'a Client,
    pub(crate) request: &'a ApiRequest,
    pub(crate) environment: &'a ApiEnvironment,
    pub(crate) headers: &'a mut Vec<(String, String)>,
    pub(crate) body: &'a [u8],
    pub(crate) url: &'a Url,
    pub(crate) method: &'a str,
}

pub(crate) async fn apply_auth(input: AuthRequest<'_>) -> Result<(), ApiError> {
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
            crate::signing::sign_aws_v4(
                method,
                url,
                headers,
                body,
                &crate::signing::AwsSigV4Params {
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
