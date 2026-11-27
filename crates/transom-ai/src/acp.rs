//! ACP（Agent Client Protocol）外部编程 Agent（Claude Code / Codex / Kimi CLI）stdio 客户端。
//!
//! 设计原理（Why）：
//! - 通过标准 ACP JSON-RPC 2.0 over stdio 在侧边栏原生驱动外部编程 Agent，同时可把 Transom
//!   自身的 `--mcp-stdio` 服务作为 MCP Server 声明给外部 Agent，使其复用已连接的 SSH 会话；
//! - 子进程统一配置 `kill_on_drop(true)`，并在取消、完成或异常退出时显式执行 `kill` + `wait`
//!   收割进程控制块，杜绝僵尸进程与脱缰孤儿进程。

use std::collections::HashMap;
use std::env;
use std::io::ErrorKind;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::acp_parse::{
    AcpStreamChunk, format_acp_rpc_error, is_retryable_acp_error, parse_acp_update,
    pick_acp_allow_option, wait_response, write_json_line,
};
use crate::acp_probe::augmented_path_env;
pub use crate::acp_probe::{acp_tools_bin_dir, is_acp_command_available};
use crate::config::AcpAgentConfig;
use crate::error::AiError;
use crate::policy::{ExecutionMode, ToolRisk, classify_shell_command};
use crate::runner::{AgentEvent, ApprovalRequest, TurnOutput};
use crate::tools::plan::PlanSnapshot;
use crate::tools::{ApprovalSummary, ToolExecutionCard};

const ACP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const ACP_TURN_TIMEOUT: Duration = Duration::from_secs(300);

/// 启动一次 ACP 外部 Agent 对话的输入参数。
pub struct AcpRunInput {
    pub agent: AcpAgentConfig,
    pub prompt: String,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub mcp_stdio_exe: Option<String>,
    pub events: mpsc::Sender<AgentEvent>,
    pub card_tx: mpsc::Sender<ToolExecutionCard>,
    pub plan_tx: Option<mpsc::Sender<PlanSnapshot>>,
    pub execution_mode: Arc<parking_lot::RwLock<ExecutionMode>>,
    pub cancel: CancellationToken,
}

struct AcpPendingTool {
    started_at: Instant,
    title: Option<String>,
    kind: Option<String>,
    input_summary: Option<String>,
    output: Option<String>,
}

/// 通过 ACP stdio 协议执行单轮外部 Coding Agent 对话并流式回传事件。
///
/// # Errors
/// 进程启动失败、握手超时、JSON-RPC 错误或用户取消时返回 [`AiError`]。
pub async fn run_acp_turn(input: AcpRunInput) -> Result<(), AiError> {
    if input.agent.command.trim().is_empty() {
        return Err(AiError::ConfigNotReady {
            field: "acp_agent.command",
        });
    }

    let mut cmd = Command::new(&input.agent.command);
    cmd.args(&input.agent.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(path_env) = augmented_path_env() {
        cmd.env("PATH", path_env);
    }
    if let Some(dir) = &input.cwd
        && !dir.trim().is_empty()
    {
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn().map_err(|err| {
        let reason = if err.kind() == ErrorKind::NotFound {
            format!(
                "未在系统 PATH 中找到可执行文件 `{}`。请先安装对应 CLI 工具并在设置中确认命令路径，或切换回“内置 AI”模式",
                input.agent.command
            )
        } else {
            format!("启动外部 ACP Agent（{}）失败: {err}", input.agent.command)
        };
        AiError::AcpFailed {
            agent: input.agent.name.clone(),
            reason,
        }
    })?;

    let mut stdin = child.stdin.take().ok_or_else(|| AiError::AcpFailed {
        agent: input.agent.name.clone(),
        reason: "无法获取 ACP 子进程 stdin".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| AiError::AcpFailed {
        agent: input.agent.name.clone(),
        reason: "无法获取 ACP 子进程 stdout".to_owned(),
    })?;
    let mut lines = BufReader::new(stdout).lines();

    let result = drive_acp_protocol(&input, &mut stdin, &mut lines).await;

    // 显式杀死并收割子进程，彻底避免僵尸进程残留。
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    result
}

async fn drive_acp_protocol(
    input: &AcpRunInput,
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> Result<(), AiError> {
    let server_name = &input.agent.name;
    write_json_line(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "transom", "version": env!("CARGO_PKG_VERSION") }
            }
        }),
        server_name,
    )
    .await?;
    let _ = wait_response(lines, 1, ACP_HANDSHAKE_TIMEOUT, &input.cancel, server_name).await?;

    let mcp_servers = match &input.mcp_stdio_exe {
        Some(exe) if !exe.trim().is_empty() => json!([{
            "name": "transom",
            "command": exe,
            "args": ["--mcp-stdio"],
            "env": []
        }]),
        _ => json!([]),
    };
    write_json_line(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": {
                "cwd": input.cwd.clone().unwrap_or_else(|| ".".to_owned()),
                "mcpServers": mcp_servers
            }
        }),
        server_name,
    )
    .await?;
    let new_resp =
        wait_response(lines, 2, ACP_HANDSHAKE_TIMEOUT, &input.cancel, server_name).await?;
    let session_id = new_resp
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_owned();

    if let Some(model_id) = input
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        let _ = write_json_line(
            stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "session/set_model",
                "params": {
                    "sessionId": session_id,
                    "modelId": model_id
                }
            }),
            server_name,
        )
        .await;
    }

    const MAX_RETRIES: usize = 2;
    let mut prompt_req_id: i64 = 3;
    let mut attempt = 0usize;

    let turn_started = Instant::now();
    let mut first_delta: Option<Instant> = None;
    let mut usage_result: Option<Value> = None;
    let mut text_buf = String::new();
    let mut reasoning_buf = String::new();
    let mut pending_tools: HashMap<String, AcpPendingTool> = HashMap::new();
    let mut completed_tools = 0usize;

    'retry: loop {
        write_json_line(
            stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": prompt_req_id,
                "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": input.prompt }]
                }
            }),
            server_name,
        )
        .await?;

        loop {
            if input.cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let next = tokio::select! {
                _ = input.cancel.cancelled() => return Err(AiError::Cancelled),
                res = tokio::time::timeout(ACP_TURN_TIMEOUT, lines.next_line()) => {
                    res.map_err(|_| AiError::Timeout {
                        operation: format!("ACP Agent ({server_name}) 响应"),
                        timeout_secs: ACP_TURN_TIMEOUT.as_secs(),
                    })?
                }
            };

            let Some(line) = next.map_err(|err| AiError::AcpFailed {
                agent: server_name.clone(),
                reason: err.to_string(),
            })?
            else {
                break 'retry;
            };

            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };

            if msg.get("id").and_then(Value::as_i64) == Some(prompt_req_id) {
                if let Some(err) = msg.get("error") {
                    let no_output = text_buf.trim().is_empty()
                        && reasoning_buf.trim().is_empty()
                        && completed_tools == 0;
                    if no_output && attempt < MAX_RETRIES && is_retryable_acp_error(err) {
                        attempt += 1;
                        prompt_req_id += 1;
                        text_buf.clear();
                        reasoning_buf.clear();
                        pending_tools.clear();
                        let wait_secs = 2u64 * attempt as u64;
                        log::warn!(
                            "ACP Agent（{server_name}）触发上游限流或并发上限，{wait_secs}s 后执行第 {attempt} 次退避重试"
                        );
                        tokio::select! {
                            _ = input.cancel.cancelled() => return Err(AiError::Cancelled),
                            _ = tokio::time::sleep(Duration::from_secs(wait_secs)) => {}
                        }
                        continue 'retry;
                    }
                    return Err(AiError::AcpFailed {
                        agent: server_name.clone(),
                        reason: format_acp_rpc_error(err, attempt > 0),
                    });
                }
                usage_result = msg.pointer("/result/usage").cloned();
                break 'retry;
            }

            let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
            if method == "session/update"
                && let Some(params) = msg.get("params")
                && let Some(chunk) = parse_acp_update(params)
            {
                match chunk {
                    AcpStreamChunk::Text(t) => {
                        first_delta.get_or_insert_with(Instant::now);
                        text_buf.push_str(&t);
                        let _ = input.events.send(AgentEvent::TextDelta(t)).await;
                    }
                    AcpStreamChunk::Reasoning(r) => {
                        first_delta.get_or_insert_with(Instant::now);
                        reasoning_buf.push_str(&r);
                        let _ = input.events.send(AgentEvent::ReasoningDelta(r)).await;
                    }
                    AcpStreamChunk::ToolCallUpdate {
                        tool_call_id,
                        title,
                        kind,
                        status,
                        input_summary,
                        output,
                    } => {
                        let entry =
                            pending_tools
                                .entry(tool_call_id.clone())
                                .or_insert_with(|| AcpPendingTool {
                                    started_at: Instant::now(),
                                    title: None,
                                    kind: None,
                                    input_summary: None,
                                    output: None,
                                });
                        if title.is_some() {
                            entry.title = title;
                        }
                        if kind.is_some() {
                            entry.kind = kind;
                        }
                        if input_summary.is_some() {
                            entry.input_summary = input_summary;
                        }
                        if output.is_some() {
                            entry.output = output;
                        }
                        if status == "completed" || status == "failed" {
                            completed_tools += 1;
                            let done =
                                pending_tools
                                    .remove(&tool_call_id)
                                    .unwrap_or(AcpPendingTool {
                                        started_at: Instant::now(),
                                        title: None,
                                        kind: None,
                                        input_summary: None,
                                        output: None,
                                    });
                            let tool_label = done.kind.unwrap_or_else(|| input.agent.id.clone());
                            let summary = done
                                .title
                                .or(done.input_summary)
                                .unwrap_or_else(|| "外部 Agent 工具".to_owned());
                            let out_text = done.output.unwrap_or_else(|| format!("状态: {status}"));
                            let _ = input.card_tx.try_send(ToolExecutionCard {
                                tool_name: tool_label,
                                host_label: input.agent.name.clone(),
                                risk: ToolRisk::Write,
                                summary,
                                output: out_text,
                                ok: status == "completed",
                                approved: true,
                                duration_ms: done.started_at.elapsed().as_millis() as u64,
                                diff_preview: None,
                            });
                        }
                    }
                    AcpStreamChunk::Plan(steps) => {
                        if let Some(tx) = &input.plan_tx {
                            let _ = tx.try_send(PlanSnapshot {
                                title: None,
                                steps,
                                focus: None,
                            });
                        }
                    }
                }
            } else if method == "session/request_permission"
                && let Some(req_id) = msg.get("id").cloned()
            {
                let title = msg
                    .pointer("/params/toolCall/title")
                    .and_then(Value::as_str)
                    .unwrap_or("外部 ACP 工具调用请求")
                    .to_owned();
                let tool_kind = msg
                    .pointer("/params/toolCall/kind")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let option_id = pick_acp_allow_option(msg.pointer("/params/options"));
                let mode = *input.execution_mode.read();
                let risk = classify_shell_command(&title);
                let auto_approve = match mode {
                    ExecutionMode::AllowAll => true,
                    ExecutionMode::Auto => risk != ToolRisk::Destructive,
                    ExecutionMode::SafeReadAuto => {
                        matches!(tool_kind, "read" | "search" | "fetch")
                            || risk == ToolRisk::SafeRead
                    }
                    ExecutionMode::Manual => false,
                };
                let approved = if auto_approve {
                    true
                } else {
                    let (tx, rx) = oneshot::channel();
                    let summary = ApprovalSummary {
                        tool_name: format!("acp:{}", input.agent.id),
                        host_label: input.agent.name.clone(),
                        protection: crate::policy::HostProtection::Normal,
                        risk,
                        title: format!("{} 请求执行权限", input.agent.name),
                        detail: title,
                        diff_preview: None,
                        requires_fresh_approval: false,
                    };
                    let _ = input
                        .events
                        .send(AgentEvent::Approval(ApprovalRequest {
                            summary,
                            suggested_prefix: None,
                            impact_note: None,
                            respond: tx,
                        }))
                        .await;
                    tokio::select! {
                        _ = input.cancel.cancelled() => return Err(AiError::Cancelled),
                        res = rx => matches!(res, Ok(transom_core::AiApprovalDecision::AllowOnce | transom_core::AiApprovalDecision::AllowPrefix { .. })),
                    }
                };
                let reply = if approved {
                    json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": { "outcome": { "outcome": "selected", "optionId": option_id } }
                    })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": { "outcome": { "outcome": "cancelled" } }
                    })
                };
                write_json_line(stdin, &reply, server_name).await?;
            }
        }
    }

    if text_buf.trim().is_empty() && reasoning_buf.trim().is_empty() && completed_tools == 0 {
        return Err(AiError::AcpFailed {
            agent: server_name.clone(),
            reason: "外部 Agent 未返回任何内容（可能是命令无法识别或模型端无响应）".to_owned(),
        });
    }

    let model_duration_ms = turn_started.elapsed().as_millis() as u64;
    let ttft_ms = first_delta.map(|f| f.saturating_duration_since(turn_started).as_millis() as u64);
    let est_in = ((input.prompt.chars().count() as u64) / 3).max(1);
    let est_out = (((text_buf.chars().count() + reasoning_buf.chars().count()) as u64) / 3).max(1);
    let (input_tokens, cached_input_tokens, output_tokens) = match usage_result {
        Some(ref u) => {
            let i = u
                .get("inputTokens")
                .or_else(|| u.get("input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(est_in);
            let c = u
                .get("cachedReadTokens")
                .or_else(|| u.get("cached_input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let o = u
                .get("outputTokens")
                .or_else(|| u.get("output_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(est_out);
            (i, c, o)
        }
        None => (est_in, 0, est_out),
    };
    let stream_secs = first_delta
        .map(|f| f.elapsed().as_secs_f32())
        .unwrap_or_else(|| (model_duration_ms as f32) / 1000.0);
    let tokens_per_sec = (stream_secs > 0.05).then(|| (output_tokens as f32) / stream_secs);

    let _ = input
        .events
        .send(AgentEvent::TurnEnd(TurnOutput {
            reasoning: reasoning_buf,
            text: text_buf,
            tokens_per_sec,
            input_tokens,
            cached_input_tokens,
            output_tokens,
            model_duration_ms,
            ttft_ms,
        }))
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_acp_command_available;

    #[test]
    fn checks_acp_command_availability() {
        assert!(!is_acp_command_available(""));
        assert!(!is_acp_command_available("   "));
        assert!(!is_acp_command_available(
            "definitely-nonexistent-acp-bin-xyz"
        ));
        assert!(is_acp_command_available("sh"));
        assert!(is_acp_command_available("/bin/sh"));
    }
}
