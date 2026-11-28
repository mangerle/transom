//! 模型列表自动发现、连通性测试与上下文窗口自动推断。
//!
//! 设计原理（Why）：
//! - `reqwest::Client` 底层自带连接池与 `Arc` 封装，统一复用 `crate::shared_http_client()`
//!   进程级单例，严禁在每次请求中临时创建 `Client::new()`；
//! - 兼容 OpenAI（`/models`）、Anthropic（`/v1/models`）与 Ollama（`/api/tags`）三种响应格式，
//!   并在缺少上下文窗口声明时依据模型名称自动推断。

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::ProviderProtocol;
use crate::error::AiError;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
/// 模型列表响应允许读取的最大字节数，防止异常端点耗尽进程内存。
const MAX_DISCOVERY_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

/// 探测到的单个远端模型摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub id: String,
    pub context_window: Option<u32>,
}

/// 供应商连通性测试结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionTestResult {
    pub latency_ms: u64,
    pub model_count: usize,
    pub detail: String,
}

/// 根据模型标识名推断常见上下文窗口大小（Tokens）。
pub fn infer_context_window(model_id: &str) -> Option<u32> {
    let lower = model_id.to_ascii_lowercase();
    if lower.contains("gemini-2") || lower.contains("gemini-1.5") {
        Some(1_048_576)
    } else if lower.contains("gpt-4.1") {
        Some(1_047_576)
    } else if lower.contains("claude-")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.contains("/o3")
    {
        Some(200_000)
    } else if lower.contains("gpt-4o")
        || lower.contains("gpt-4-turbo")
        || lower.contains("deepseek")
        || lower.contains("qwen")
        || lower.contains("llama-3.3")
        || lower.contains("llama3.3")
    {
        Some(128_000)
    } else if lower.contains("gpt-3.5") {
        Some(16_384)
    } else {
        None
    }
}

/// 从 OpenAI / Anthropic / Ollama 的模型列表 JSON 中提取模型条目。
pub fn parse_models_payload(value: &serde_json::Value) -> Vec<DiscoveredModel> {
    let mut models = Vec::new();
    if let Some(arr) = value.get("data").and_then(serde_json::Value::as_array) {
        models.reserve(arr.len());
        for item in arr {
            if let Some(id) = item.get("id").and_then(serde_json::Value::as_str) {
                let ctx = item
                    .get("context_length")
                    .or_else(|| item.get("context_window"))
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .or_else(|| infer_context_window(id));
                models.push(DiscoveredModel {
                    id: id.to_owned(),
                    context_window: ctx,
                });
            }
        }
    } else if let Some(arr) = value.get("models").and_then(serde_json::Value::as_array) {
        models.reserve(arr.len());
        for item in arr {
            if let Some(name) = item
                .get("name")
                .or_else(|| item.get("model"))
                .and_then(serde_json::Value::as_str)
            {
                models.push(DiscoveredModel {
                    id: name.to_owned(),
                    context_window: infer_context_window(name),
                });
            }
        }
    }
    models
}

/// 向指定供应商拉取可用模型列表。
///
/// # Errors
/// 地址无效、网络超时或接口返回非 2xx 状态时返回 [`AiError::ModelCallFailed`]。
pub async fn discover_models(
    protocol: ProviderProtocol,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<DiscoveredModel>, AiError> {
    let url = build_models_url(protocol, base_url)?;
    let client = crate::shared_http_client().map_err(|reason| AiError::ModelCallFailed {
        protocol: protocol.as_str().to_owned(),
        model: "models".to_owned(),
        reason,
    })?;
    let mut req = client.get(&url).timeout(DISCOVERY_TIMEOUT);
    match protocol {
        ProviderProtocol::Anthropic => {
            if !api_key.trim().is_empty() {
                req = req.header("x-api-key", api_key.trim());
            }
            req = req.header("anthropic-version", "2023-06-01");
        }
        ProviderProtocol::OpenAi | ProviderProtocol::OpenAiResponses => {
            if !api_key.trim().is_empty() {
                req = req.bearer_auth(api_key.trim());
            }
        }
    }

    let resp = tokio::time::timeout(DISCOVERY_TIMEOUT, req.send())
        .await
        .map_err(|_| AiError::Timeout {
            operation: "拉取模型列表".to_owned(),
            timeout_secs: DISCOVERY_TIMEOUT.as_secs(),
        })?
        .map_err(|err| AiError::ModelCallFailed {
            protocol: protocol.as_str().to_owned(),
            model: "models".to_owned(),
            reason: err.to_string(),
        })?;

    let status = resp.status();
    let body_text = tokio::time::timeout(
        DISCOVERY_TIMEOUT,
        read_bounded_response_body(resp, protocol),
    )
    .await
    .map_err(|_| AiError::Timeout {
        operation: "读取模型列表响应".to_owned(),
        timeout_secs: DISCOVERY_TIMEOUT.as_secs(),
    })??;

    if !status.is_success() {
        return Err(AiError::ModelCallFailed {
            protocol: protocol.as_str().to_owned(),
            model: "models".to_owned(),
            reason: format!("HTTP {status}: {body_text}"),
        });
    }

    let json: serde_json::Value =
        serde_json::from_str(&body_text).map_err(|err| AiError::ModelCallFailed {
            protocol: protocol.as_str().to_owned(),
            model: "models".to_owned(),
            reason: format!("解析模型列表 JSON 失败: {err}"),
        })?;
    Ok(parse_models_payload(&json))
}

async fn read_bounded_response_body(
    resp: reqwest::Response,
    protocol: ProviderProtocol,
) -> Result<String, AiError> {
    if let Some(length) = resp.content_length()
        && length > MAX_DISCOVERY_RESPONSE_BYTES
    {
        return Err(response_too_large(protocol, length));
    }

    let initial_capacity = resp
        .content_length()
        .unwrap_or(0)
        .min(MAX_DISCOVERY_RESPONSE_BYTES) as usize;
    let mut body = Vec::with_capacity(initial_capacity);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|err| AiError::ModelCallFailed {
            protocol: protocol.as_str().to_owned(),
            model: "models".to_owned(),
            reason: format!("读取模型列表响应流失败: {err}"),
        })?;
        append_bounded_chunk(&mut body, &chunk, protocol)?;
    }
    String::from_utf8(body).map_err(|err| AiError::ModelCallFailed {
        protocol: protocol.as_str().to_owned(),
        model: "models".to_owned(),
        reason: format!("模型列表响应不是有效 UTF-8: {err}"),
    })
}

fn append_bounded_chunk(
    body: &mut Vec<u8>,
    chunk: &[u8],
    protocol: ProviderProtocol,
) -> Result<(), AiError> {
    let next_len = body
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| response_too_large(protocol, u64::MAX))?;
    if next_len as u64 > MAX_DISCOVERY_RESPONSE_BYTES {
        return Err(response_too_large(protocol, next_len as u64));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn response_too_large(protocol: ProviderProtocol, actual_bytes: u64) -> AiError {
    AiError::ModelCallFailed {
        protocol: protocol.as_str().to_owned(),
        model: "models".to_owned(),
        reason: format!(
            "模型列表响应超过体积上限（实际 {actual_bytes} 字节，上限 {MAX_DISCOVERY_RESPONSE_BYTES} 字节）"
        ),
    }
}

/// 测试供应商端点连通性并统计往返延迟与可用模型数。
///
/// # Errors
/// 网络失败或鉴权失败时返回 [`AiError::ModelCallFailed`]。
pub async fn test_provider_connection(
    protocol: ProviderProtocol,
    base_url: &str,
    api_key: &str,
) -> Result<ConnectionTestResult, AiError> {
    let started = Instant::now();
    let models = discover_models(protocol, base_url, api_key).await?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let model_count = models.len();
    let detail = format!("连接成功（延迟 {latency_ms}ms，发现 {model_count} 个模型）");
    Ok(ConnectionTestResult {
        latency_ms,
        model_count,
        detail,
    })
}

fn build_models_url(protocol: ProviderProtocol, raw_base: &str) -> Result<String, AiError> {
    let trimmed = raw_base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AiError::ConfigNotReady { field: "base_url" });
    }
    if trimmed.ends_with("/models") || trimmed.ends_with("/api/tags") {
        return Ok(trimmed.to_owned());
    }
    if protocol == ProviderProtocol::Anthropic && !trimmed.ends_with("/v1") {
        Ok(format!("{trimmed}/v1/models"))
    } else {
        Ok(format!("{trimmed}/models"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        MAX_DISCOVERY_RESPONSE_BYTES, append_bounded_chunk, infer_context_window,
        parse_models_payload,
    };
    use crate::config::ProviderProtocol;

    #[test]
    fn infers_known_context_windows() {
        assert_eq!(infer_context_window("claude-sonnet-4-5"), Some(200_000));
        assert_eq!(infer_context_window("gpt-4.1-mini"), Some(1_047_576));
        assert_eq!(infer_context_window("deepseek-chat"), Some(128_000));
        assert_eq!(infer_context_window("unknown-custom"), None);
    }

    #[test]
    fn parses_openai_and_ollama_models_payloads() {
        let openai = json!({
            "data": [
                { "id": "gpt-4o" },
                { "id": "local-qwen", "context_length": 32768 }
            ]
        });
        let parsed = parse_models_payload(&openai);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].context_window, Some(128_000));
        assert_eq!(parsed[1].context_window, Some(32_768));

        let ollama = json!({
            "models": [{ "name": "qwen2.5:14b" }]
        });
        let parsed_ollama = parse_models_payload(&ollama);
        assert_eq!(parsed_ollama.len(), 1);
        assert_eq!(parsed_ollama[0].id, "qwen2.5:14b");
        assert_eq!(parsed_ollama[0].context_window, Some(128_000));
    }

    #[test]
    fn rejects_out_of_range_context_windows_without_wrapping() {
        let payload = json!({
            "data": [
                { "id": "unknown-custom", "context_length": u64::MAX },
                { "id": "gpt-4o", "context_length": u64::MAX }
            ]
        });
        let parsed = parse_models_payload(&payload);
        assert_eq!(parsed[0].context_window, None);
        assert_eq!(parsed[1].context_window, Some(128_000));
    }

    #[test]
    fn discovery_response_limit_is_bounded() {
        assert_eq!(MAX_DISCOVERY_RESPONSE_BYTES, 2 * 1024 * 1024);

        let mut body = vec![0; MAX_DISCOVERY_RESPONSE_BYTES as usize];
        assert!(
            append_bounded_chunk(&mut body, b"x", ProviderProtocol::OpenAi).is_err(),
            "超过上限的流式分块必须立即拒绝"
        );
    }
}
