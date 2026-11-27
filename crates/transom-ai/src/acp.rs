//! ACP（Agent Client Protocol）外部编程 Agent（Claude Code / Codex / Kimi CLI）stdio 客户端。
//!
//! 设计原理（Why）：
//! - 通过标准 ACP JSON-RPC 2.0 over stdio 在侧边栏原生驱动外部编程 Agent，同时可把 Transom
//!   自身的 `--mcp-stdio` 服务作为 MCP Server 声明给外部 Agent，使其复用已连接的 SSH 会话；
//! - 子进程统一配置 `kill_on_drop(true)`，并在取消、完成或异常退出时显式执行 `kill` + `wait`
//!   收割进程控制块，杜绝僵尸进程与脱缰孤儿进程。

use std::collections::HashSet;
use std::env;
use std::io::ErrorKind;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::acp_handlers::{
    AcpTurnState, finalize_acp_turn, handle_acp_permission, handle_acp_update,
};
use crate::acp_parse::{
    format_acp_rpc_error, is_retryable_acp_error, parse_acp_update, wait_response, write_json_line,
};
use crate::acp_probe::augmented_path_env;
pub use crate::acp_probe::{acp_tools_bin_dir, is_acp_command_available};
use crate::config::AcpAgentConfig;
use crate::error::AiError;
use crate::exec_policy::ExecPolicyRule;
use crate::policy::{ExecutionMode, HostProtection};
use crate::runner::AgentEvent;
use crate::tools::ToolExecutionCard;
use crate::tools::plan::PlanSnapshot;

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
    pub protection: HostProtection,
    pub plan_mode: bool,
    pub exec_policy_rules: Vec<ExecPolicyRule>,
    pub session_allow_prefixes: Arc<parking_lot::RwLock<HashSet<String>>>,
    pub cancel: CancellationToken,
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

async fn initialize_acp_session(
    input: &AcpRunInput,
    stdin: &mut ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<ChildStdout>>,
) -> Result<String, AiError> {
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
    Ok(session_id)
}

async fn drive_acp_protocol(
    input: &AcpRunInput,
    stdin: &mut ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<ChildStdout>>,
) -> Result<(), AiError> {
    let server_name = &input.agent.name;
    let session_id = initialize_acp_session(input, stdin, lines).await?;

    const MAX_RETRIES: usize = 2;
    let mut prompt_req_id: i64 = 3;
    let mut attempt = 0usize;
    let mut state = AcpTurnState::new();

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
                    if !state.has_any_output()
                        && attempt < MAX_RETRIES
                        && is_retryable_acp_error(err)
                    {
                        attempt += 1;
                        prompt_req_id += 1;
                        state.reset_for_retry();
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
                state.usage_result = msg.pointer("/result/usage").cloned();
                break 'retry;
            }

            let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
            if method == "session/update"
                && let Some(params) = msg.get("params")
                && let Some(chunk) = parse_acp_update(params)
            {
                handle_acp_update(input, &mut state, chunk).await;
            } else if method == "session/request_permission"
                && let Some(req_id) = msg.get("id").cloned()
            {
                handle_acp_permission(input, stdin, &msg, req_id).await?;
            }
        }
    }

    finalize_acp_turn(input, state).await
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
        #[cfg(unix)]
        {
            assert!(is_acp_command_available("sh"));
            assert!(is_acp_command_available("/bin/sh"));
        }
        #[cfg(windows)]
        {
            assert!(is_acp_command_available("cmd"));
            assert!(is_acp_command_available("cmd.exe"));
        }
    }
}
