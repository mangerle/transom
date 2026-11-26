//! HTTP 请求体编码、表单字段与文件上载。

use super::http::http_auth::{AuthRequest, apply_auth};
use super::http::{MAX_REQUEST_TIMEOUT_SECONDS, MAX_UPLOAD_BYTES, apply_cookies};
use super::http_support::{
    ensure_header, ensure_header_if_absent, header_name, header_value, method_for, read_response,
    request_host, safe_url_label,
};
use super::{
    ApiAuth, ApiBodyType, ApiCookie, ApiEnvironment, ApiError, ApiFormFieldKind, ApiRequest,
    ApiResponseSnapshot,
};
use reqwest::multipart::{Form, Part};
use reqwest::{Body, Client, Url};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;

/// HTTP 或 GraphQL 请求在应用认证与 Cookie 前的已编码部分。
pub(super) struct PreparedRequestParts {
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: PreparedBody,
}

pub(super) enum PreparedBody {
    Bytes(Vec<u8>),
    File { path: String },
}

impl PreparedBody {
    pub(super) fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::File { .. } => None,
        }
    }

    pub(super) async fn into_reqwest_body(self) -> Result<Body, ApiError> {
        match self {
            Self::Bytes(bytes) => Ok(Body::from(bytes)),
            Self::File { path } => {
                let file =
                    tokio::fs::File::open(&path)
                        .await
                        .map_err(|error| ApiError::UploadFile {
                            path: path.clone(),
                            reason: error.to_string(),
                        })?;
                Ok(Body::from(file))
            }
        }
    }

    pub(super) async fn sha256_hex(&self) -> Result<Option<String>, ApiError> {
        let Self::File { path } = self else {
            return Ok(None);
        };
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| ApiError::UploadFile {
                path: path.clone(),
                reason: error.to_string(),
            })?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| ApiError::UploadFile {
                    path: path.clone(),
                    reason: error.to_string(),
                })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(Some(hex::encode(hasher.finalize())))
    }
}

pub(super) async fn prepared_http_parts(
    request: &ApiRequest,
    graphql: bool,
) -> Result<PreparedRequestParts, ApiError> {
    let mut headers = request
        .headers
        .iter()
        .filter(|header| header.enabled)
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect::<Vec<_>>();
    if graphql {
        ensure_header(&mut headers, "content-type", "application/json");
        let variables: serde_json::Value = serde_json::from_str(&request.graphql_variables)
            .map_err(|error| ApiError::InvalidGraphQlVariables {
                reason: error.to_string(),
            })?;
        let body = serde_json::json!({
            "query": request.body,
            "variables": variables
        });
        return Ok(PreparedRequestParts {
            headers,
            body: PreparedBody::Bytes(serde_json::to_vec(&body).map_err(|error| {
                ApiError::InvalidGraphQlVariables {
                    reason: error.to_string(),
                }
            })?),
        });
    }
    if let Some(content_type) = raw_content_type(request.body_type) {
        ensure_header_if_absent(&mut headers, "content-type", content_type);
    }
    let body = match request.body_type {
        ApiBodyType::None => PreparedBody::Bytes(Vec::new()),
        ApiBodyType::Binary => {
            let path = upload_path(&request.body_file);
            let length = upload_file_length(&path).await?;
            if !request.headers.iter().any(|header| {
                header.enabled && header.name.trim().eq_ignore_ascii_case("content-type")
            }) {
                let mime = mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .to_string();
                ensure_header(&mut headers, "content-type", &mime);
            }
            ensure_header_if_absent(&mut headers, "content-length", &length.to_string());
            PreparedBody::File { path }
        }
        ApiBodyType::Json
        | ApiBodyType::Text
        | ApiBodyType::Xml
        | ApiBodyType::JavaScript
        | ApiBodyType::Html => PreparedBody::Bytes(request.body.as_bytes().to_vec()),
        ApiBodyType::FormUrlEncoded | ApiBodyType::Multipart => {
            return Err(ApiError::InvalidBody {
                reason: "表单请求体不应进入原始请求体构建路径".to_string(),
            });
        }
    };
    Ok(PreparedRequestParts { headers, body })
}

fn raw_content_type(body_type: ApiBodyType) -> Option<&'static str> {
    match body_type {
        ApiBodyType::Json => Some("application/json"),
        ApiBodyType::Xml => Some("application/xml"),
        ApiBodyType::JavaScript => Some("application/javascript"),
        ApiBodyType::Html => Some("text/html"),
        ApiBodyType::Text => Some("text/plain; charset=utf-8"),
        ApiBodyType::None
        | ApiBodyType::FormUrlEncoded
        | ApiBodyType::Multipart
        | ApiBodyType::Binary => None,
    }
}

pub(super) async fn send_form_http(
    client: &Client,
    token_client: &Client,
    request: &ApiRequest,
    environment: &ApiEnvironment,
    cookies: &[ApiCookie],
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
    if matches!(request.auth, ApiAuth::AwsSigV4 { .. }) {
        return Err(ApiError::InvalidAuth {
            reason: "AWS SigV4 暂不支持表单或 multipart 请求体".to_string(),
        });
    }
    let host = request_host(&url);
    let method = method_for(request.method);
    let mut headers = request
        .headers
        .iter()
        .filter(|header| header.enabled)
        .filter(|header| !header.name.trim().eq_ignore_ascii_case("content-type"))
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect::<Vec<_>>();
    apply_cookies(&url, cookies, &mut headers);
    apply_auth(AuthRequest {
        token_client,
        request,
        environment,
        headers: &mut headers,
        body: &[],
        payload_hash: None,
        url: &url,
        method: method.as_str(),
    })
    .await?;
    let mut builder = client.request(method, url);
    if request.timeout_seconds > 0 {
        builder = builder.timeout(Duration::from_secs(
            request.timeout_seconds.min(MAX_REQUEST_TIMEOUT_SECONDS),
        ));
    }
    for header in &headers {
        builder = builder.header(header_name(header)?, header_value(header)?);
    }
    builder = if request.body_type == ApiBodyType::Multipart {
        let form = build_multipart_form(request).await?;
        builder.multipart(form)
    } else {
        let fields = request
            .form_fields
            .iter()
            .filter(|field| field.enabled && !field.name.trim().is_empty())
            .map(|field| (field.name.trim().to_string(), field.value.clone()))
            .collect::<Vec<_>>();
        builder.form(&fields)
    };
    let started = Instant::now();
    let response = builder.send().await.map_err(|error| ApiError::Transport {
        host: host.clone(),
        reason: error.to_string(),
    })?;
    read_response(response, started, &host).await
}

async fn build_multipart_form(request: &ApiRequest) -> Result<Form, ApiError> {
    let mut form = Form::new();
    for field in request
        .form_fields
        .iter()
        .filter(|field| field.enabled && !field.name.trim().is_empty())
    {
        let name = field.name.trim();
        if field.kind == ApiFormFieldKind::File {
            let path = upload_path(&field.value);
            upload_file_length(&path).await?;
            let part = Part::file(&path)
                .await
                .map_err(|error| ApiError::MultipartField {
                    field: name.to_string(),
                    reason: error.to_string(),
                })?;
            form = form.part(name.to_string(), part);
        } else {
            form = form.text(name.to_string(), field.value.clone());
        }
    }
    Ok(form)
}

fn upload_path(value: &str) -> String {
    value
        .trim()
        .strip_prefix("file://")
        .unwrap_or(value.trim())
        .to_string()
}

async fn upload_file_length(path: &str) -> Result<u64, ApiError> {
    if path.trim().is_empty() {
        return Err(ApiError::UploadFile {
            path: path.to_string(),
            reason: "未选择文件".to_string(),
        });
    }
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| ApiError::UploadFile {
            path: path.to_string(),
            reason: error.to_string(),
        })?;
    if metadata.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::UploadFileTooLarge {
            path: path.to_string(),
            limit: MAX_UPLOAD_BYTES,
        });
    }
    Ok(metadata.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn raw_language_sets_default_content_type_without_overriding_user_header() {
        let mut request = ApiRequest::new("脚本", None);
        request.body_type = ApiBodyType::JavaScript;
        let prepared = prepared_http_parts(&request, false)
            .await
            .expect("请求体应可构建");
        assert!(
            prepared
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                    && value == "application/javascript")
        );

        request.headers.push(super::super::ApiHeader::new(
            "Content-Type",
            "application/x-custom",
        ));
        let prepared = prepared_http_parts(&request, false)
            .await
            .expect("请求体应可构建");
        assert!(
            prepared
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                    && value == "application/x-custom")
        );
        assert!(
            !prepared
                .headers
                .iter()
                .any(|(_, value)| value == "application/javascript")
        );
    }
}
