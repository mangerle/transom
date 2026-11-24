//! HTTP 请求体编码、表单字段与文件上载。

use super::http::http_auth::{AuthRequest, apply_auth};
use super::http::{MAX_REQUEST_TIMEOUT_SECONDS, MAX_UPLOAD_BYTES, apply_cookies};
use super::http_support::{
    ensure_header, header_name, header_value, method_for, read_response, request_host,
    safe_url_label,
};
use super::{
    ApiAuth, ApiBodyType, ApiCookie, ApiEnvironment, ApiError, ApiFormFieldKind, ApiRequest,
    ApiResponseSnapshot,
};
use reqwest::multipart::{Form, Part};
use reqwest::{Client, Url};
use std::path::Path;
use std::time::{Duration, Instant};

/// HTTP 或 GraphQL 请求在应用认证与 Cookie 前的已编码部分。
pub(super) struct PreparedRequestParts {
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

pub(super) async fn prepared_http_parts(
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
    if request.body_type == ApiBodyType::Json {
        ensure_header(&mut headers, "content-type", "application/json");
    } else if request.body_type == ApiBodyType::Xml {
        ensure_header(&mut headers, "content-type", "application/xml");
    }
    let body = match request.body_type {
        ApiBodyType::None => Vec::new(),
        ApiBodyType::Binary => {
            let path = upload_path(&request.body_file);
            let bytes = read_upload_file(&path).await?;
            if !request.headers.iter().any(|header| {
                header.enabled && header.name.trim().eq_ignore_ascii_case("content-type")
            }) {
                let mime = mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .to_string();
                ensure_header(&mut headers, "content-type", &mime);
            }
            bytes
        }
        ApiBodyType::Json | ApiBodyType::Text | ApiBodyType::Xml => {
            request.body.as_bytes().to_vec()
        }
        ApiBodyType::FormUrlEncoded | ApiBodyType::Multipart => unreachable_marked_as_error()?,
    };
    Ok(PreparedRequestParts { headers, body })
}

fn unreachable_marked_as_error() -> Result<Vec<u8>, ApiError> {
    Err(ApiError::InvalidAuth {
        reason: "表单请求体不应进入原始请求体构建路径".to_string(),
    })
}

pub(super) async fn send_form_http(
    client: &Client,
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
        .map(|header| {
            (
                environment.resolve(&header.name),
                environment.resolve(&header.value),
            )
        })
        .collect::<Vec<_>>();
    apply_cookies(&url, cookies, &mut headers);
    apply_auth(AuthRequest {
        client,
        request,
        environment,
        headers: &mut headers,
        body: &[],
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
        let form = build_multipart_form(request, environment).await?;
        builder.multipart(form)
    } else {
        let fields = request
            .form_fields
            .iter()
            .filter(|field| field.enabled && !field.name.trim().is_empty())
            .map(|field| {
                (
                    environment.resolve(field.name.trim()),
                    environment.resolve(&field.value),
                )
            })
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

async fn build_multipart_form(
    request: &ApiRequest,
    environment: &ApiEnvironment,
) -> Result<Form, ApiError> {
    let mut form = Form::new();
    for field in request
        .form_fields
        .iter()
        .filter(|field| field.enabled && !field.name.trim().is_empty())
    {
        let name = environment.resolve(field.name.trim());
        if field.kind == ApiFormFieldKind::File {
            let path = upload_path(&environment.resolve(&field.value));
            let bytes = read_upload_file(&path).await?;
            let file_name = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or("upload.bin")
                .to_string();
            let mime = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();
            let part = Part::bytes(bytes)
                .file_name(file_name)
                .mime_str(&mime)
                .map_err(|error| ApiError::MultipartField {
                    field: name.clone(),
                    reason: error.to_string(),
                })?;
            form = form.part(name, part);
        } else {
            form = form.text(name, environment.resolve(&field.value));
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

async fn read_upload_file(path: &str) -> Result<Vec<u8>, ApiError> {
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
    tokio::fs::read(path)
        .await
        .map_err(|error| ApiError::UploadFile {
            path: path.to_string(),
            reason: error.to_string(),
        })
}
