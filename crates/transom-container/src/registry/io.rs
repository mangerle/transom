//! 有上限的响应体读取。

use super::{MAX_ERROR_BYTES, MAX_JSON_BYTES, registry_error};
use crate::error::ContainerError;
use futures::StreamExt;
use reqwest::Response;
use serde::de::DeserializeOwned;

pub(super) async fn read_json<T: DeserializeOwned>(
    response: Response,
    operation: &str,
) -> Result<T, ContainerError> {
    let status = response.status();
    if !status.is_success() {
        let body = read_error_text(response).await;
        return Err(registry_error(operation, format!("HTTP {status}: {body}")));
    }
    let bytes = read_bytes_limited(response, MAX_JSON_BYTES, operation).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| registry_error(operation, format!("响应解析失败: {error}")))
}

pub(super) async fn read_bytes_limited(
    response: Response,
    limit: usize,
    operation: &str,
) -> Result<Vec<u8>, ContainerError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(8192.min(limit));
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| registry_error(operation, error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(registry_error(
                operation,
                format!("响应超过上限 {limit} 字节"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) async fn read_error_text(response: Response) -> String {
    match read_bytes_limited(response, MAX_ERROR_BYTES, "读取仓库错误响应").await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(error) => error.to_string(),
    }
}
