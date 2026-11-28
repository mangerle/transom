//! ACP 流式事件处理、统一权限协商与单轮收尾统计。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value as JsonValue;
use serde_json::{Value, json};
use tokio::process::ChildStdin;
use tokio::sync::oneshot;

use crate::acp::AcpRunInput;
use crate::acp_parse::{AcpStreamChunk, pick_acp_allow_option, write_json_line};
use crate::error::AiError;
use crate::exec_policy::{explain_command_impact, extract_command_prefix};
use crate::policy::{
    CommandPolicyContext, PolicyDecision, ToolRisk, assess_shell_command, evaluate_command_policy,
};
use crate::runner::{AgentEvent, ApprovalRequest, TurnOutput};
use crate::tools::plan::PlanSnapshot;
use crate::tools::{ApprovalSummary, ToolExecutionCard};

pub(crate) struct AcpPendingTool {
    pub(crate) started_at: Instant,
    pub(crate) title: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) input_summary: Option<String>,
    pub(crate) output: Option<String>,
}

pub(crate) struct AcpTurnState {
    pub(crate) turn_started: Instant,
    pub(crate) first_delta: Option<Instant>,
    pub(crate) usage_result: Option<JsonValue>,
    pub(crate) text_buf: String,
    pub(crate) reasoning_buf: String,
    pub(crate) pending_tools: HashMap<String, AcpPendingTool>,
    pub(crate) completed_tools: usize,
}

impl AcpTurnState {
    pub(crate) fn new() -> Self {
        Self {
            turn_started: Instant::now(),
            first_delta: None,
            usage_result: None,
            text_buf: String::new(),
            reasoning_buf: String::new(),
            pending_tools: HashMap::new(),
            completed_tools: 0,
        }
    }

    pub(crate) fn reset_for_retry(&mut self) {
        self.text_buf.clear();
        self.reasoning_buf.clear();
        self.pending_tools.clear();
    }

    pub(crate) fn has_any_output(&self) -> bool {
        !self.text_buf.trim().is_empty()
            || !self.reasoning_buf.trim().is_empty()
            || self.completed_tools > 0
    }
}

pub(crate) async fn handle_acp_update(
    input: &AcpRunInput,
    state: &mut AcpTurnState,
    chunk: AcpStreamChunk,
) {
    match chunk {
        AcpStreamChunk::Text(t) => {
            state.first_delta.get_or_insert_with(Instant::now);
            state.text_buf.push_str(&t);
            let _ = input.events.send(AgentEvent::TextDelta(t)).await;
        }
        AcpStreamChunk::Reasoning(r) => {
            state.first_delta.get_or_insert_with(Instant::now);
            state.reasoning_buf.push_str(&r);
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
            let entry = state
                .pending_tools
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
                state.completed_tools += 1;
                let done = state
                    .pending_tools
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
                if let Err(err) = input
                    .events
                    .send(AgentEvent::ToolCard(ToolExecutionCard {
                        tool_name: tool_label,
                        host_label: input.agent.name.clone(),
                        risk: ToolRisk::Write,
                        summary,
                        output: out_text,
                        ok: status == "completed",
                        approved: true,
                        duration_ms: done.started_at.elapsed().as_millis() as u64,
                        diff_preview: None,
                        ..Default::default()
                    }))
                    .await
                {
                    log::warn!("发送 ACP 工具卡片失败: {err}");
                }
            }
        }
        AcpStreamChunk::Plan(steps) => {
            if let Err(err) = input
                .events
                .send(AgentEvent::Plan(PlanSnapshot {
                    title: None,
                    steps,
                    focus: None,
                }))
                .await
            {
                log::warn!("发送 ACP 计划更新失败: {err}");
            }
        }
    }
}

const ACP_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// ACP 权限请求的策略评估结果。
struct AcpPermissionAssessment {
    command: Option<String>,
    risk: ToolRisk,
    force_confirm: bool,
    detail: String,
}

/// 从 ACP 的结构化工具输入中提取真实命令。
///
/// `title` 是展示字段，不能作为安全判断依据。优先读取 ACP 的 `rawInput`，
/// 兼容常见的 `input`、`arguments` 和字符串载荷；字段缺失时返回 `None`，
/// 由调用方按工具类型选择 fail-closed 的默认风险。
fn extract_acp_permission_command(msg: &Value) -> Option<String> {
    const COMMAND_POINTERS: [&str; 8] = [
        "/params/toolCall/rawInput/command",
        "/params/toolCall/rawInput/cmd",
        "/params/toolCall/rawInput/shellCommand",
        "/params/toolCall/rawInput/input",
        "/params/toolCall/input/command",
        "/params/toolCall/input/cmd",
        "/params/toolCall/arguments/command",
        "/params/toolCall/arguments/cmd",
    ];
    for pointer in COMMAND_POINTERS {
        if let Some(command) = msg
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(command.to_owned());
        }
    }
    for pointer in [
        "/params/toolCall/rawInput",
        "/params/toolCall/input",
        "/params/toolCall/arguments",
    ] {
        if let Some(command) = msg
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(command.to_owned());
        }
    }
    None
}

/// 根据真实命令和 ACP 工具类型生成权限请求的风险快照。
fn assess_acp_permission(msg: &Value) -> AcpPermissionAssessment {
    let title = msg
        .pointer("/params/toolCall/title")
        .and_then(Value::as_str)
        .unwrap_or("外部 ACP 工具调用请求")
        .trim()
        .to_owned();
    let tool_kind = msg
        .pointer("/params/toolCall/kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    let command = extract_acp_permission_command(msg);
    let (risk, force_confirm) = match command.as_deref() {
        Some(command) => {
            let assessment = assess_shell_command(command);
            (assessment.risk, assessment.requires_confirmation)
        }
        None if matches!(tool_kind, "read" | "search" | "fetch") => (ToolRisk::SafeRead, false),
        None => (ToolRisk::Write, true),
    };
    AcpPermissionAssessment {
        detail: command.as_deref().unwrap_or(&title).to_owned(),
        command,
        risk,
        force_confirm,
    }
}

pub(crate) async fn handle_acp_permission(
    input: &AcpRunInput,
    stdin: &mut ChildStdin,
    msg: &Value,
    req_id: Value,
) -> Result<(), AiError> {
    let assessment = assess_acp_permission(msg);
    let option_id = pick_acp_allow_option(msg.pointer("/params/options"));
    let mode = *input.execution_mode.read();
    let decision = {
        let grants = input.approval_grants.read();
        evaluate_command_policy(&CommandPolicyContext {
            protection: input.protection,
            mode,
            plan_mode: input.plan_mode,
            base_risk: assessment.risk,
            force_confirm: assessment.force_confirm,
            command: assessment.command.as_deref(),
            exec_policy_rules: &input.exec_policy_rules,
            approval_scope: &input.approval_scope,
            approval_grants: &grants,
        })
    };
    let approved = match decision {
        PolicyDecision::Allow => true,
        PolicyDecision::Deny { .. } => false,
        PolicyDecision::RequireApproval => {
            let suggested_prefix = assessment
                .command
                .as_deref()
                .and_then(extract_command_prefix);
            let impact_note = assessment
                .command
                .as_deref()
                .and_then(explain_command_impact);
            let (tx, rx) = oneshot::channel();
            let summary = ApprovalSummary {
                tool_name: format!("acp:{}", input.agent.id),
                host_label: input.agent.name.clone(),
                protection: input.protection,
                risk: assessment.risk,
                title: format!("{} 请求执行权限", input.agent.name),
                detail: assessment.detail,
                diff_preview: None,
                requires_fresh_approval: false,
            };
            if let Err(err) = input
                .events
                .send(AgentEvent::Approval(ApprovalRequest {
                    summary,
                    suggested_prefix,
                    impact_note,
                    respond: tx,
                }))
                .await
            {
                return Err(AiError::AcpFailed {
                    agent: input.agent.name.clone(),
                    reason: format!("无法把 ACP 权限请求发送到界面: {err}"),
                });
            }
            let reply = tokio::select! {
                _ = input.cancel.cancelled() => return Err(AiError::Cancelled),
                res = tokio::time::timeout(ACP_APPROVAL_TIMEOUT, rx) => match res {
                    Ok(reply) => reply.ok(),
                    Err(_) => {
                        log::warn!(
                            "ACP Agent（{}）权限请求等待用户确认超时（{} 秒）",
                            input.agent.name,
                            ACP_APPROVAL_TIMEOUT.as_secs()
                        );
                        None
                    }
                },
            };
            match reply {
                Some(transom_core::AiApprovalDecision::AllowOnce) => true,
                Some(transom_core::AiApprovalDecision::AllowPrefix { prefix }) => {
                    if let Some(prefix) = extract_command_prefix(&prefix) {
                        input
                            .approval_grants
                            .write()
                            .grant_prefix(&input.approval_scope, &prefix);
                    }
                    true
                }
                _ => false,
            }
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
    write_json_line(stdin, &reply, &input.agent.name).await
}

pub(crate) async fn finalize_acp_turn(
    input: &AcpRunInput,
    state: AcpTurnState,
) -> Result<(), AiError> {
    if !state.has_any_output() {
        return Err(AiError::AcpFailed {
            agent: input.agent.name.clone(),
            reason: "外部 Agent 未返回任何内容（可能是命令无法识别或模型端无响应）".to_owned(),
        });
    }

    let model_duration_ms = state.turn_started.elapsed().as_millis() as u64;
    let ttft_ms = state
        .first_delta
        .map(|f| f.saturating_duration_since(state.turn_started).as_millis() as u64);
    let est_in = ((input.prompt.chars().count() as u64) / 3).max(1);
    let est_out = (((state.text_buf.chars().count() + state.reasoning_buf.chars().count()) as u64)
        / 3)
    .max(1);
    let (input_tokens, cached_input_tokens, output_tokens) = match state.usage_result {
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
    let stream_secs = state
        .first_delta
        .map(|f| f.elapsed().as_secs_f32())
        .unwrap_or_else(|| (model_duration_ms as f32) / 1000.0);
    let tokens_per_sec = (stream_secs > 0.05).then(|| (output_tokens as f32) / stream_secs);

    let _ = input
        .events
        .send(AgentEvent::TurnEnd(TurnOutput {
            reasoning: state.reasoning_buf,
            text: state.text_buf,
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
    use serde_json::json;

    use super::assess_acp_permission;
    use crate::policy::ToolRisk;

    #[test]
    fn uses_structured_command_instead_of_misleading_title() {
        let msg = json!({
            "params": {
                "toolCall": {
                    "title": "读取配置文件",
                    "kind": "read",
                    "rawInput": { "command": "rm -rf /tmp/prod" }
                }
            }
        });
        let assessment = assess_acp_permission(&msg);
        assert_eq!(assessment.command.as_deref(), Some("rm -rf /tmp/prod"));
        assert_eq!(assessment.risk, ToolRisk::Destructive);
    }

    #[test]
    fn missing_execute_command_fails_closed() {
        let msg = json!({
            "params": {
                "toolCall": {
                    "title": "执行检查",
                    "kind": "execute",
                    "rawInput": {}
                }
            }
        });
        let assessment = assess_acp_permission(&msg);
        assert_eq!(assessment.risk, ToolRisk::Write);
        assert!(assessment.force_confirm);
    }

    #[test]
    fn read_without_command_remains_read_only() {
        let msg = json!({
            "params": {
                "toolCall": {
                    "title": "读取配置",
                    "kind": "read",
                    "rawInput": {}
                }
            }
        });
        let assessment = assess_acp_permission(&msg);
        assert_eq!(assessment.risk, ToolRisk::SafeRead);
        assert!(!assessment.force_confirm);
    }
}
