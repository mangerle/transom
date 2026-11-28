//! ACP `session/update` 通知解析：提取文本增量、思考增量、真实工具调用状态与计划步骤。

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncWriteExt as _, BufReader};
use tokio_util::sync::CancellationToken;

use crate::protocol_lines::{MAX_PROTOCOL_LINE_BYTES, read_bounded_line};
use crate::tools::plan::{PlanStep, PlanStepStatus};
use transom_ai_contracts::error::AiError;

pub(crate) fn acp_err(agent: &str, err: impl ToString) -> AiError {
    AiError::AcpFailed {
        agent: agent.to_owned(),
        reason: err.to_string(),
    }
}

pub(crate) async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    payload: &Value,
    server_name: &str,
) -> Result<(), AiError> {
    let line = serde_json::to_string(payload).map_err(|e| acp_err(server_name, e))?;
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| acp_err(server_name, e))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|e| acp_err(server_name, e))?;
    stdin.flush().await.map_err(|e| acp_err(server_name, e))
}

/// 解析后的 ACP 流式更新事件。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AcpStreamChunk {
    Text(String),
    Reasoning(String),
    ToolCallUpdate {
        tool_call_id: String,
        title: Option<String>,
        kind: Option<String>,
        status: String,
        input_summary: Option<String>,
        output: Option<String>,
    },
    Plan(Vec<PlanStep>),
}

fn non_empty_str(val: Option<&Value>) -> Option<String> {
    val.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// 保留流式增量文本中的原始空白字符与换行，仅过滤完全空字符串。
fn raw_non_empty_str(val: Option<&Value>) -> Option<String> {
    val.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn summarize_raw_input(raw: &Value) -> Option<String> {
    for key in [
        "command",
        "description",
        "path",
        "file_path",
        "query",
        "url",
    ] {
        if let Some(s) = non_empty_str(raw.get(key)) {
            return Some(s.chars().take(120).collect());
        }
    }
    if let Some(s) = raw.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(s.chars().take(120).collect());
    }
    None
}

fn extract_tool_output(update: &Value) -> Option<String> {
    if let Some(arr) = update.get("content").and_then(Value::as_array) {
        let mut texts = Vec::new();
        for item in arr {
            if let Some(t) = non_empty_str(item.pointer("/content/text"))
                .or_else(|| non_empty_str(item.get("text")))
            {
                texts.push(t);
            }
        }
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    if let Some(raw_out) = update.get("rawOutput") {
        if let Some(s) = non_empty_str(Some(raw_out)) {
            return Some(s);
        }
        if let Some(s) = non_empty_str(raw_out.get("output"))
            .or_else(|| non_empty_str(raw_out.get("stdout")))
            .or_else(|| non_empty_str(raw_out.get("message")))
        {
            return Some(s);
        }
    }
    None
}

/// 解析 ACP `session/update` 通知载荷并转换为内部流式事件。
pub(crate) fn parse_acp_update(params: &Value) -> Option<AcpStreamChunk> {
    let update = params.get("update").unwrap_or(params);
    let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
    match kind {
        "agent_message_chunk" => {
            raw_non_empty_str(update.pointer("/content/text")).map(AcpStreamChunk::Text)
        }
        "agent_thought_chunk" => {
            raw_non_empty_str(update.pointer("/content/text")).map(AcpStreamChunk::Reasoning)
        }
        "tool_call" | "tool_call_update" => {
            let tool_call_id = non_empty_str(update.get("toolCallId"))
                .or_else(|| non_empty_str(update.get("id")))
                .unwrap_or_else(|| "default".to_owned());
            let title = non_empty_str(update.get("title"));
            let tool_kind = non_empty_str(update.get("kind"));
            let status =
                non_empty_str(update.get("status")).unwrap_or_else(|| "running".to_owned());
            let input_summary = update.get("rawInput").and_then(summarize_raw_input);
            let output = extract_tool_output(update);
            Some(AcpStreamChunk::ToolCallUpdate {
                tool_call_id,
                title,
                kind: tool_kind,
                status,
                input_summary,
                output,
            })
        }
        "plan" => {
            let entries = update.get("entries").and_then(Value::as_array)?;
            let steps: Vec<PlanStep> = entries
                .iter()
                .filter_map(|e| {
                    let text = non_empty_str(e.get("content"))
                        .or_else(|| non_empty_str(e.get("title")))?;
                    let status = match e.get("status").and_then(Value::as_str).unwrap_or("") {
                        "completed" | "done" => PlanStepStatus::Done,
                        "in_progress" => PlanStepStatus::InProgress,
                        "failed" => PlanStepStatus::Failed,
                        _ => PlanStepStatus::Pending,
                    };
                    Some(PlanStep { text, status })
                })
                .collect();
            (!steps.is_empty()).then_some(AcpStreamChunk::Plan(steps))
        }
        _ => None,
    }
}

/// 从 ACP `session/request_permission` 的候选项中优先挑选自动批准选项 ID。
pub(crate) fn pick_acp_allow_option(options: Option<&Value>) -> String {
    if let Some(arr) = options.and_then(Value::as_array) {
        for preferred in ["allow_always", "allow_once"] {
            if let Some(id) = arr
                .iter()
                .find(|o| o.get("kind").and_then(Value::as_str) == Some(preferred))
                .and_then(|o| o.get("optionId"))
                .and_then(Value::as_str)
            {
                return id.to_owned();
            }
        }
        if let Some(id) = arr
            .first()
            .and_then(|o| o.get("optionId"))
            .and_then(Value::as_str)
        {
            return id.to_owned();
        }
    }
    "allow".to_owned()
}

/// 判断 ACP JSON-RPC 错误是否属于上游并发限制或临时限流，可进行退避重试。
pub(crate) fn is_retryable_acp_error(err: &Value) -> bool {
    let lower = err.to_string().to_lowercase();
    [
        "concurrency limit",
        "tier_limit_exceeded",
        "rate_limit",
        "too many requests",
        "overloaded",
        "429",
        "retry after",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

/// 将 ACP JSON-RPC 错误对象（含嵌套 API JSON）转换为简洁可读的中文提示。
pub(crate) fn format_acp_rpc_error(err: &Value, retried: bool) -> String {
    let raw_msg = err
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| err.to_string());
    let inner_msg = extract_nested_error_message(&raw_msg).unwrap_or_else(|| raw_msg.clone());
    let lower = format!("{raw_msg} {inner_msg}").to_lowercase();

    if lower.contains("concurrency limit") || lower.contains("tier_limit_exceeded") {
        if retried {
            "上游 API 并发数已达套餐上限（已自动重试仍繁忙），请等待其他在途请求完成后重试，或切换至内置模型".to_owned()
        } else {
            "上游 API 并发数已达套餐上限，请等待其他在途请求完成后重试，或切换至内置模型".to_owned()
        }
    } else if lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("429")
    {
        "上游 API 触发频率限制，请稍后重试".to_owned()
    } else if lower.contains("overloaded") || lower.contains("529") {
        "上游模型服务暂时过载，请稍后重试".to_owned()
    } else if lower.contains("401")
        || lower.contains("authentication")
        || lower.contains("invalid_api_key")
    {
        "外部 Agent API 密钥无效或已过期，请检查对应 CLI 登录与配置".to_owned()
    } else if lower.contains("insufficient_quota")
        || lower.contains("credit")
        || lower.contains("balance")
    {
        "上游 API 账户额度不足，请检查账单或切换模型".to_owned()
    } else {
        inner_msg
    }
}

fn extract_nested_error_message(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let parsed: Value = serde_json::from_str(&raw[start..]).ok()?;
    parsed
        .pointer("/error/message")
        .or_else(|| parsed.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) async fn wait_response_raw(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    expected_id: i64,
    timeout: Duration,
    cancel: &CancellationToken,
    server_name: &str,
) -> Result<Value, AiError> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            return Err(AiError::Timeout {
                operation: format!("等待 ACP ({server_name}) 握手响应"),
                timeout_secs: timeout.as_secs(),
            });
        }
        let line = tokio::select! {
            _ = cancel.cancelled() => return Err(AiError::Cancelled),
            res = tokio::time::timeout(
                remain,
                read_bounded_line(reader, MAX_PROTOCOL_LINE_BYTES),
            ) => {
                res.map_err(|_| AiError::Timeout {
                    operation: format!("等待 ACP ({server_name}) 握手响应"),
                    timeout_secs: timeout.as_secs(),
                })?
            }
        }
        .map_err(|err| AiError::AcpFailed {
            agent: server_name.to_owned(),
            reason: err.to_string(),
        })?;

        let Some(line) = line else {
            return Err(AiError::AcpFailed {
                agent: server_name.to_owned(),
                reason: "ACP 子进程提前退出".to_owned(),
            });
        };
        if let Ok(val) = serde_json::from_str::<Value>(&line)
            && val.get("id").and_then(Value::as_i64) == Some(expected_id)
        {
            return Ok(val);
        }
    }
}

pub(crate) async fn wait_response(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    expected_id: i64,
    timeout: Duration,
    cancel: &CancellationToken,
    server_name: &str,
) -> Result<Value, AiError> {
    let val = wait_response_raw(reader, expected_id, timeout, cancel, server_name).await?;
    if let Some(err) = val.get("error") {
        return Err(AiError::AcpFailed {
            agent: server_name.to_owned(),
            reason: format_acp_rpc_error(err, false),
        });
    }
    Ok(val)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AcpStreamChunk, format_acp_rpc_error, is_retryable_acp_error, parse_acp_update};
    use crate::tools::plan::{PlanStep, PlanStepStatus};

    #[test]
    fn parses_text_thought_plan_and_tool_details() {
        let msg = json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hello" }
            }
        });
        assert_eq!(
            parse_acp_update(&msg),
            Some(AcpStreamChunk::Text("hello".to_owned()))
        );

        let ws_chunk = json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": " world\n  " }
            }
        });
        assert_eq!(
            parse_acp_update(&ws_chunk),
            Some(AcpStreamChunk::Text(" world\n  ".to_owned()))
        );

        let newline_thought = json!({
            "update": {
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "\n" }
            }
        });
        assert_eq!(
            parse_acp_update(&newline_thought),
            Some(AcpStreamChunk::Reasoning("\n".to_owned()))
        );

        let tool_start = json!({
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call-1",
                "title": "Run df -h",
                "kind": "execute",
                "status": "in_progress",
                "rawInput": { "command": "df -h /" }
            }
        });
        assert_eq!(
            parse_acp_update(&tool_start),
            Some(AcpStreamChunk::ToolCallUpdate {
                tool_call_id: "call-1".to_owned(),
                title: Some("Run df -h".to_owned()),
                kind: Some("execute".to_owned()),
                status: "in_progress".to_owned(),
                input_summary: Some("df -h /".to_owned()),
                output: None,
            })
        );

        let plan = json!({
            "update": {
                "sessionUpdate": "plan",
                "entries": [
                    { "content": "检查磁盘", "status": "completed" },
                    { "content": "分析目录", "status": "in_progress" }
                ]
            }
        });
        assert_eq!(
            parse_acp_update(&plan),
            Some(AcpStreamChunk::Plan(vec![
                PlanStep {
                    text: "检查磁盘".to_owned(),
                    status: PlanStepStatus::Done,
                },
                PlanStep {
                    text: "分析目录".to_owned(),
                    status: PlanStepStatus::InProgress,
                },
            ]))
        );
    }

    #[test]
    fn formats_nested_concurrency_and_rate_limit_errors() {
        let concurrency_err = json!({
            "code": -32603,
            "message": r#"Internal error: API Error: 400 {"type":"error","error":{"type":"invalid_request_error","message":"Plan concurrency limit exceeded (10). Retry after an in-flight request finishes.","code":"tier_limit_exceeded"}}"#
        });
        assert!(is_retryable_acp_error(&concurrency_err));
        let formatted = format_acp_rpc_error(&concurrency_err, true);
        assert!(formatted.contains("并发数已达套餐上限"));
        assert!(formatted.contains("已自动重试仍繁忙"));

        let rate_err = json!({ "code": -32000, "message": "429 Too Many Requests" });
        assert!(is_retryable_acp_error(&rate_err));
        assert!(format_acp_rpc_error(&rate_err, false).contains("频率限制"));
    }
}
