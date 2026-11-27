//! Agent 运行期生命周期钩子 [`Gate`]：Plan 拦截、链式命令策略、生命周期 Hooks 与终端接管。

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use rig_agent::agent::{
    AgentHook, CompletionCallAction, CompletionCallEvent, DispatchAction, DispatchEvent,
    HookContext as RigHookContext, ModelTurnAction, ModelTurnFinished, ObservationAction,
    OutcomeAction, OutcomeEvent, ReasoningDelta, TextDelta,
};
use rig_core::message::AssistantContent;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_core::AiApprovalDecision;

use crate::exec_policy::{
    ExecPolicyRule, evaluate_command_chain, explain_command_impact, extract_command_prefix,
};
use crate::hooks::{HookContext, HookEvent, HookExecutor};
use crate::policy::{
    ExecutionMode, HostProtection, PolicyDecision, ToolRisk, detect_blocking_command,
    evaluate_bulk_policy, evaluate_policy, format_blocking_command_hint,
};
use crate::runner::{AgentEvent, ApprovalRequest, HandoffRequest, TurnOutput};
use crate::tools::{ToolRiskAssessment, ToolRuntimeContext, assess_tool_call_value};

/// 运行期生命周期钩子：熔断、Plan 护栏、策略引擎、Hooks 与流式增量转发。
pub(crate) struct Gate {
    pub(crate) events: mpsc::Sender<AgentEvent>,
    pub(crate) tool_ctx: ToolRuntimeContext,
    pub(crate) execution_mode: Arc<RwLock<ExecutionMode>>,
    pub(crate) plan_mode: bool,
    pub(crate) exec_policy_rules: Vec<ExecPolicyRule>,
    pub(crate) session_allow_prefixes: Arc<RwLock<HashSet<String>>>,
    pub(crate) hook_executor: Option<Arc<dyn HookExecutor>>,
    pub(crate) max_turns: usize,
    pub(crate) turns_used: AtomicUsize,
    pub(crate) cancel: CancellationToken,
    pub(crate) steer_queue: Arc<Mutex<VecDeque<String>>>,
    pub(crate) call_started: Mutex<Option<Instant>>,
    pub(crate) first_delta: Mutex<Option<Instant>>,
    pub(crate) last_summary: Arc<Mutex<String>>,
}

impl Gate {
    fn mark_delta(&self) {
        let mut guard = self.first_delta.lock();
        guard.get_or_insert_with(Instant::now);
    }

    fn drain_steer_notes(&self) -> Option<String> {
        let mut q = self.steer_queue.lock();
        if q.is_empty() {
            return None;
        }
        let items: Vec<String> = q.drain(..).collect();
        Some(items.join("\n"))
    }
}

impl AgentHook for Gate {
    async fn on_completion_call(
        &self,
        _ctx: &RigHookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.cancel.is_cancelled() {
            return CompletionCallAction::stop("用户已取消本次提问");
        }
        let turn = self.turns_used.fetch_add(1, Ordering::Relaxed) + 1;
        if turn > self.max_turns {
            return CompletionCallAction::stop(format!(
                "已达到最大工具调用轮次上限（{} 轮）",
                self.max_turns
            ));
        }
        *self.call_started.lock() = Some(Instant::now());
        *self.first_delta.lock() = None;
        CompletionCallAction::continue_run()
    }

    async fn on_text_delta(
        &self,
        _ctx: &RigHookContext,
        delta: TextDelta<'_>,
    ) -> ObservationAction {
        if self.cancel.is_cancelled() {
            return ObservationAction::stop("用户已取消");
        }
        self.mark_delta();
        let _ = self
            .events
            .send(AgentEvent::TextDelta(delta.delta.to_owned()))
            .await;
        ObservationAction::continue_run()
    }

    async fn on_reasoning_delta(
        &self,
        _ctx: &RigHookContext,
        delta: ReasoningDelta<'_>,
    ) -> ObservationAction {
        if self.cancel.is_cancelled() {
            return ObservationAction::stop("用户已取消");
        }
        self.mark_delta();
        let _ = self
            .events
            .send(AgentEvent::ReasoningDelta(delta.delta.to_owned()))
            .await;
        ObservationAction::continue_run()
    }

    async fn on_model_turn_finished(
        &self,
        _ctx: &RigHookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let mut reasoning_parts = Vec::new();
        let mut text_parts = Vec::new();
        for item in event.content {
            match item {
                AssistantContent::Reasoning(r) => {
                    if !r.text.trim().is_empty() {
                        reasoning_parts.push(r.text.clone());
                    }
                }
                AssistantContent::Text(t) if !t.text.trim().is_empty() => {
                    text_parts.push(t.text.clone());
                }
                _ => {}
            }
        }
        let call_start = self.call_started.lock().take();
        let first_delta = self.first_delta.lock().take();
        let model_duration_ms = call_start
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let ttft_ms = call_start
            .zip(first_delta)
            .map(|(s, f)| f.saturating_duration_since(s).as_millis() as u64);
        let elapsed = first_delta.map(|start| start.elapsed());
        let tokens_per_sec = compute_tokens_per_sec(event.usage.output_tokens, elapsed);
        let input_tokens = event.usage.input_tokens.unwrap_or(0);
        let cached_input_tokens = event.usage.cached_input_tokens.unwrap_or(0);
        let output_tokens = event.usage.output_tokens.unwrap_or(0);
        let _ = self
            .events
            .send(AgentEvent::TurnEnd(TurnOutput {
                reasoning: reasoning_parts.join("\n\n"),
                text: text_parts.join("\n\n"),
                tokens_per_sec,
                input_tokens,
                cached_input_tokens,
                output_tokens,
                model_duration_ms,
                ttft_ms,
            }))
            .await;

        if !text_parts.is_empty() {
            *self.last_summary.lock() = text_parts.join("\n").chars().take(240).collect();
        }
        if let Some(steer) = self.drain_steer_notes() {
            return ModelTurnAction::retry_with_feedback(format!(
                "[用户中途引导补充]\n{steer}\n请立即按上述补充调整并继续执行。"
            ));
        }
        ModelTurnAction::continue_run()
    }

    async fn on_outcome(&self, _ctx: &RigHookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        if let Some(result) = event.tool_result()
            && let Some(steer) = self.drain_steer_notes()
        {
            let merged = format!(
                "{}\n\n[用户中途引导补充]\n{steer}",
                result.output().as_text().unwrap_or_default()
            );
            return OutcomeAction::rewrite_tool_result(&event, merged);
        }
        OutcomeAction::proceed()
    }

    async fn on_dispatch(&self, _ctx: &RigHookContext, event: DispatchEvent<'_>) -> DispatchAction {
        if self.cancel.is_cancelled() {
            return DispatchAction::stop("用户已取消");
        }
        let Some(tool_name) = event.tool_name() else {
            return DispatchAction::proceed();
        };
        let args_json = event.tool_args().unwrap_or("{}");
        let parsed: serde_json::Value =
            serde_json::from_str(args_json).unwrap_or(serde_json::Value::Null);
        let cmd_str = parsed["command"].as_str();

        if matches!(tool_name, "remote_exec" | "remote_exec_bulk")
            && let Some(cmd) = cmd_str
            && let Some(hit) = detect_blocking_command(cmd)
        {
            return DispatchAction::skip(format_blocking_command_hint(&hit));
        }

        let assessment = assess_tool_call_value(tool_name, &parsed, args_json, &self.tool_ctx);
        if tool_name != "update_plan" {
            let _ = self.events.try_send(AgentEvent::FocusUpdated(format!(
                "正在执行 {}…",
                assessment.summary.title
            )));
        }

        if let Some(blocked) = self
            .check_preconditions(tool_name, args_json, &assessment)
            .await
        {
            return blocked;
        }

        let decision = match self.decide_policy(tool_name, &parsed, cmd_str, &assessment) {
            Ok(d) => d,
            Err(skip_action) => return skip_action,
        };

        match decision {
            PolicyDecision::Allow => DispatchAction::proceed(),
            PolicyDecision::Deny { reason } => DispatchAction::skip(reason),
            PolicyDecision::RequireApproval => {
                self.await_approval_or_handoff(assessment, cmd_str).await
            }
        }
    }
}

impl Gate {
    /// 检查 Plan 只读模式硬拦截与 `PreToolUse` 生命周期钩子。
    async fn check_preconditions(
        &self,
        tool_name: &str,
        args_json: &str,
        assessment: &ToolRiskAssessment,
    ) -> Option<DispatchAction> {
        if self.plan_mode && matches!(assessment.risk, ToolRisk::Write | ToolRisk::Destructive) {
            return Some(DispatchAction::skip(
                "当前处于只读规划（Plan）模式，已拦截变更操作。请仅执行只读探测，并通过 `update_plan` 输出完整方案等待用户确认后再执行。",
            ));
        }
        if let Some(exec) = &self.hook_executor {
            let hook_ctx = HookContext {
                tool_name: Some(tool_name.to_owned()),
                args_json: Some(args_json.to_owned()),
                target_label: assessment.summary.host_label.clone(),
                tool_ok: None,
                summary: assessment.summary.detail.clone(),
            };
            let outcome = exec.execute(HookEvent::PreToolUse, &hook_ctx).await;
            if let Some(reason) = outcome.blocked_reason {
                return Some(DispatchAction::skip(format!(
                    "PreToolUse 钩子拦截：{reason}"
                )));
            }
        }
        None
    }

    /// 结合命令链策略、批量主机策略与基础保护级别计算最终裁决。
    fn decide_policy(
        &self,
        tool_name: &str,
        parsed: &serde_json::Value,
        cmd_str: Option<&str>,
        assessment: &ToolRiskAssessment,
    ) -> Result<PolicyDecision, DispatchAction> {
        let mode = *self.execution_mode.read();
        let mut effective_risk = assessment.risk;
        let mut chain_allowlisted = false;
        let mut force_ask = assessment.force_confirm;

        if matches!(tool_name, "remote_exec" | "remote_exec_bulk")
            && let Some(cmd) = cmd_str
        {
            let chain = {
                let prefixes = self.session_allow_prefixes.read();
                evaluate_command_chain(cmd, &self.exec_policy_rules, &prefixes)
            };
            if let Some(denied) = chain.denied_reason {
                return Err(DispatchAction::skip(denied));
            }
            if chain.worst_risk > effective_risk {
                effective_risk = chain.worst_risk;
            }
            chain_allowlisted = chain.all_allowlisted;
            force_ask = force_ask || chain.force_ask;
        }

        if tool_name == "remote_exec_bulk" {
            let requested: Option<Vec<&str>> = parsed["host_names"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());
            let targets: Vec<HostProtection> = self
                .tool_ctx
                .bulk_hosts
                .iter()
                .filter(|h| match &requested {
                    Some(names) if !names.is_empty() => names.contains(&h.host_name.as_str()),
                    _ => true,
                })
                .map(|h| h.protection)
                .collect();
            return Ok(evaluate_bulk_policy(&targets, mode, effective_risk).0);
        }

        let base = evaluate_policy(self.tool_ctx.protection(), mode, effective_risk, force_ask);
        if base == PolicyDecision::RequireApproval
            && chain_allowlisted
            && self.tool_ctx.protection() != HostProtection::ReadOnly
            && effective_risk != ToolRisk::Destructive
        {
            Ok(PolicyDecision::Allow)
        } else {
            Ok(base)
        }
    }

    /// 挂起等待用户审批决策；若用户选择移交终端，则进一步等待终端接管结果。
    async fn await_approval_or_handoff(
        &self,
        assessment: ToolRiskAssessment,
        cmd_str: Option<&str>,
    ) -> DispatchAction {
        let suggested_prefix = cmd_str.and_then(extract_command_prefix);
        let impact_note = cmd_str.and_then(explain_command_impact);
        let command_preview = cmd_str
            .map(str::to_owned)
            .unwrap_or_else(|| assessment.summary.detail.clone());
        let host_label = assessment.summary.host_label.clone();

        let (tx, rx) = oneshot::channel();
        if self
            .events
            .send(AgentEvent::Approval(ApprovalRequest {
                summary: assessment.summary,
                suggested_prefix,
                impact_note,
                respond: tx,
            }))
            .await
            .is_err()
        {
            return DispatchAction::stop("事件通道已关闭");
        }
        let reply = tokio::select! {
            _ = self.cancel.cancelled() => return DispatchAction::stop("用户已取消"),
            res = rx => match res {
                Ok(val) => val,
                Err(_) => return DispatchAction::stop("审批通道已断开"),
            }
        };
        match reply {
            AiApprovalDecision::AllowOnce => DispatchAction::proceed(),
            AiApprovalDecision::AllowPrefix { prefix } => {
                if !prefix.trim().is_empty() {
                    self.session_allow_prefixes.write().insert(prefix);
                }
                DispatchAction::proceed()
            }
            AiApprovalDecision::RejectWithFeedback { reason } => {
                let note = if reason.trim().is_empty() {
                    "用户拒绝执行该操作，请调整方案或向用户确认后再试。".to_owned()
                } else {
                    format!("用户拒绝了该操作，并附带修改意见：{reason}。请据此调整方案。")
                };
                DispatchAction::skip(note)
            }
            AiApprovalDecision::HandoffToTerminal => {
                self.await_terminal_handoff(host_label, command_preview)
                    .await
            }
        }
    }

    /// 挂起等待用户在交互式终端中手动完成操作并回传结果。
    async fn await_terminal_handoff(
        &self,
        host_label: String,
        command_preview: String,
    ) -> DispatchAction {
        let (htx, hrx) = oneshot::channel();
        if self
            .events
            .send(AgentEvent::TerminalHandoff(HandoffRequest {
                host_label,
                command: command_preview,
                respond: htx,
            }))
            .await
            .is_err()
        {
            return DispatchAction::stop("终端接管通道已关闭");
        }
        let handoff_res = tokio::select! {
            _ = self.cancel.cancelled() => return DispatchAction::stop("用户已取消"),
            res = hrx => res.ok(),
        };
        match handoff_res {
            Some(Ok(Some(out))) if !out.trim().is_empty() => DispatchAction::skip(format!(
                "该步骤已由用户在交互式终端手动完成，终端输出摘要如下：\n{out}\n请基于此结果继续后续步骤，勿重复执行该命令。"
            )),
            Some(Ok(_)) => DispatchAction::skip(
                "该步骤已由用户在交互式终端手动完成，请直接继续后续步骤，勿重复执行该命令。",
            ),
            _ => DispatchAction::skip("用户取消了终端手动接管，请调整方案。"),
        }
    }
}

fn compute_tokens_per_sec(tokens: Option<u64>, elapsed: Option<Duration>) -> Option<f32> {
    let tokens = tokens?;
    let elapsed = elapsed?;
    let secs = elapsed.as_secs_f64();
    if tokens == 0 || secs < 0.05 {
        return None;
    }
    Some((tokens as f64 / secs) as f32)
}
