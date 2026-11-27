//! AI 运行时事件泵、增量节流合并与 JSONL / 审计落盘。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AcpAgentConfig, AcpRunInput, AgentEvent, AiAuditEntry, ChatRole, HookContext, HookEvent,
    HookExecutor, MemorySnapshot, PlanSnapshot, PlanStepStatus, RunInput, SessionEntry,
    SessionEntryKind, ToolExecutionCard, ToolRisk, TurnUsageRecord, compute_context_breakdown,
    run_acp_turn,
};
use transom_core::{
    AiAppEvent, AiApprovalDecision, AiApprovalDto, AiPlanDto, AiPlanStepDto, AiPlanStepStatus,
    AiToolCardDto, AiTurnMetricsDto, AppEvent,
};
use transom_store::AiStore;
use uuid::Uuid;

use super::event_pump::{AgentPumpSink, drive_agent_pump};

/// 控制平面启动一次 AI 运行后的事件泵参数。
pub(super) struct PumpContext {
    pub(super) run_id: String,
    pub(super) conversation_id: String,
    pub(super) host_label: String,
    pub(super) model_name: String,
    pub(super) store: Arc<AiStore>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) approvals: Arc<Mutex<HashMap<String, oneshot::Sender<AiApprovalDecision>>>>,
    pub(super) handoffs: super::super::state::HandoffSenderMap,
    pub(super) runs_map: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub(super) steers_map: super::super::state::SteerQueueMap,
    pub(super) title: String,
    pub(super) acp_agent: Option<AcpAgentConfig>,
    pub(super) hook_executor: Option<Arc<dyn HookExecutor>>,
    pub(super) preamble: String,
    pub(super) memory: MemorySnapshot,
    pub(super) context_window: u32,
    pub(super) history_entries: Vec<SessionEntry>,
    pub(super) current_turn_entry_id: Arc<parking_lot::RwLock<Option<String>>>,
}

/// 驱动 `transom_ai::run` 并同步泵送事件回 UI，同时负责 JSONL 与审计落盘。
pub(super) async fn execute_and_pump(
    input: RunInput,
    card_rx: mpsc::Receiver<ToolExecutionCard>,
    plan_rx: mpsc::Receiver<PlanSnapshot>,
    event_rx: mpsc::Receiver<AgentEvent>,
    mut ctx: PumpContext,
) {
    let mut last_leaf_id = input.history_branch.last().map(|entry| entry.id.clone());

    // 先落盘用户提问条目
    let user_entry = SessionEntry::new(
        last_leaf_id.clone(),
        SessionEntryKind::Message {
            role: ChatRole::User,
            text: input.prompt.clone(),
            images: Vec::new(),
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    last_leaf_id = Some(user_entry.id.clone());
    *ctx.current_turn_entry_id.write() = Some(user_entry.id.clone());
    {
        let store = ctx.store.clone();
        let conv_id = ctx.conversation_id.clone();
        let entry_clone = user_entry.clone();
        if let Ok(Err(err)) = tokio::task::spawn_blocking(move || {
            store.append_session_entry(&conv_id, &entry_clone)
        })
        .await
        {
            log::warn!("追加 AI 用户提问条目失败: {err}");
        }
    }
    ctx.history_entries.push(user_entry.clone());
    let _ = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::RunStarted {
            run_id: ctx.run_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            title: ctx.title.clone(),
            user_entry_id: user_entry.id.clone(),
        })))
        .await;

    let runner_cancel = input.cancel.clone();
    let runner_handle = if let Some(agent) = ctx.acp_agent.clone() {
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string());
        let mut acp_prompt = match &input.active_skill_prompt {
            Some(tmpl) => format!("{tmpl}\n\n用户请求：{}", input.prompt),
            None => input.prompt.clone(),
        };
        if let Some(inst) = input
            .response_language
            .as_deref()
            .and_then(transom_ai::language_instruction)
        {
            acp_prompt = format!("[语言要求] {inst}\n\n{acp_prompt}");
        }
        let acp_input = AcpRunInput {
            agent,
            prompt: acp_prompt,
            model: (!input.target.model.is_empty()).then(|| input.target.model.clone()),
            cwd: None,
            mcp_stdio_exe: exe,
            events: input.events.clone(),
            card_tx: input.tool_ctx.card_tx.clone(),
            plan_tx: input.tool_ctx.plan_tx.clone(),
            execution_mode: input.execution_mode.clone(),
            protection: input.tool_ctx.protection(),
            plan_mode: input.plan_mode,
            exec_policy_rules: input.exec_policy_rules.clone(),
            session_allow_prefixes: input.session_allow_prefixes.clone(),
            cancel: input.cancel.clone(),
        };
        tokio::spawn(async move { run_acp_turn(acp_input).await })
    } else {
        tokio::spawn(async move { transom_ai::run(input).await })
    };

    let mut sink = MainAgentPumpSink {
        ctx,
        last_leaf_id,
        text_buf: String::new(),
        reasoning_buf: String::new(),
    };
    let runner_err = drive_agent_pump(
        Some(runner_handle),
        event_rx,
        card_rx,
        Some(plan_rx),
        Some(Duration::from_millis(25)),
        "AI 运行任务异常中断",
        &mut sink,
    )
    .await;

    let MainAgentPumpSink {
        ctx, last_leaf_id, ..
    } = sink;

    // 更新会话索引的 active_leaf_id 与时间戳
    {
        let store = ctx.store.clone();
        let conv_id = ctx.conversation_id.clone();
        let now = chrono::Utc::now().to_rfc3339();
        let _ = tokio::task::spawn_blocking(move || {
            store.update_conversation_leaf(&conv_id, last_leaf_id, now)
        })
        .await;
    }

    let cancelled = runner_cancel.is_cancelled()
        || runner_err
            .as_deref()
            .is_some_and(|e| e.contains("已取消") || e.contains("Cancelled"));
    ctx.runs_map.lock().remove(&ctx.run_id);
    ctx.steers_map.lock().remove(&ctx.run_id);
    let _ = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::RunFinished {
            run_id: ctx.run_id,
            conversation_id: ctx.conversation_id,
            error: runner_err,
            cancelled,
        })))
        .await;
}

struct MainAgentPumpSink {
    ctx: PumpContext,
    last_leaf_id: Option<String>,
    text_buf: String,
    reasoning_buf: String,
}

#[async_trait]
impl AgentPumpSink for MainAgentPumpSink {
    async fn on_tick(&mut self) {
        flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
    }

    async fn on_card(&mut self, card: ToolExecutionCard) {
        handle_tool_card(&mut self.ctx, &mut self.last_leaf_id, card).await;
    }

    async fn on_plan(&mut self, plan: PlanSnapshot) {
        let dto = AiPlanDto {
            title: plan.title,
            focus: plan.focus,
            steps: plan
                .steps
                .into_iter()
                .map(|s| AiPlanStepDto {
                    text: s.text,
                    status: match s.status {
                        PlanStepStatus::Pending => AiPlanStepStatus::Pending,
                        PlanStepStatus::InProgress => AiPlanStepStatus::InProgress,
                        PlanStepStatus::Done => AiPlanStepStatus::Done,
                        PlanStepStatus::Failed => AiPlanStepStatus::Failed,
                    },
                })
                .collect(),
        };
        let _ = self
            .ctx
            .event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::PlanUpdated {
                run_id: self.ctx.run_id.clone(),
                conversation_id: self.ctx.conversation_id.clone(),
                plan: dto,
            })))
            .await;
    }

    async fn on_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta(d) => self.text_buf.push_str(&d),
            AgentEvent::ReasoningDelta(d) => self.reasoning_buf.push_str(&d),
            AgentEvent::FocusUpdated(focus) => {
                let _ = self
                    .ctx
                    .event_tx
                    .send_async(AppEvent::Ai(Box::new(AiAppEvent::FocusUpdated {
                        run_id: self.ctx.run_id.clone(),
                        focus,
                    })))
                    .await;
            }
            AgentEvent::TurnEnd(turn) => {
                flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
                handle_turn_end(&mut self.ctx, &mut self.last_leaf_id, turn).await;
            }
            AgentEvent::Approval(req) => {
                flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
                let request_id = Uuid::new_v4().to_string();
                self.ctx
                    .approvals
                    .lock()
                    .insert(request_id.clone(), req.respond);
                let dto = AiApprovalDto {
                    request_id,
                    run_id: self.ctx.run_id.clone(),
                    tool_name: req.summary.tool_name,
                    host_label: req.summary.host_label,
                    protection: req.summary.protection,
                    risk_label: req.summary.risk.label().to_owned(),
                    is_destructive: req.summary.risk == ToolRisk::Destructive,
                    title: req.summary.title,
                    detail: req.summary.detail,
                    diff_preview: req.summary.diff_preview,
                    requires_fresh_approval: req.summary.requires_fresh_approval,
                    suggested_prefix: req.suggested_prefix,
                    impact_note: req.impact_note,
                };
                let _ = self
                    .ctx
                    .event_tx
                    .send_async(AppEvent::Ai(Box::new(AiAppEvent::ApprovalRequested(dto))))
                    .await;
            }
            AgentEvent::TerminalHandoff(req) => {
                flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
                self.ctx
                    .handoffs
                    .lock()
                    .insert(self.ctx.run_id.clone(), req.respond);
                let _ = self
                    .ctx
                    .event_tx
                    .send_async(AppEvent::Ai(Box::new(
                        AiAppEvent::TerminalHandoffRequested {
                            run_id: self.ctx.run_id.clone(),
                            host_label: req.host_label,
                            command: req.command,
                        },
                    )))
                    .await;
            }
        }
    }
}

async fn flush_deltas(ctx: &PumpContext, text_buf: &mut String, reasoning_buf: &mut String) {
    if !reasoning_buf.is_empty() {
        let delta = std::mem::take(reasoning_buf);
        let _ = ctx
            .event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::ReasoningDelta {
                run_id: ctx.run_id.clone(),
                conversation_id: ctx.conversation_id.clone(),
                delta,
            })))
            .await;
    }
    if !text_buf.is_empty() {
        let delta = std::mem::take(text_buf);
        let _ = ctx
            .event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::TextDelta {
                run_id: ctx.run_id.clone(),
                conversation_id: ctx.conversation_id.clone(),
                delta,
            })))
            .await;
    }
}

async fn handle_turn_end(
    ctx: &mut PumpContext,
    last_leaf_id: &mut Option<String>,
    turn: transom_ai::TurnOutput,
) {
    let mut to_append = Vec::with_capacity(2);
    if !turn.reasoning.trim().is_empty() {
        let entry = SessionEntry::new(
            last_leaf_id.clone(),
            SessionEntryKind::Reasoning {
                text: turn.reasoning.clone(),
            },
        );
        *last_leaf_id = Some(entry.id.clone());
        to_append.push(entry.clone());
        ctx.history_entries.push(entry);
    }
    if !turn.text.trim().is_empty() {
        let entry = SessionEntry::new(
            last_leaf_id.clone(),
            SessionEntryKind::Message {
                role: ChatRole::Assistant,
                text: turn.text.clone(),
                images: Vec::new(),
                model: Some(ctx.model_name.clone()),
                tokens_per_sec: turn.tokens_per_sec,
                usage: Some(TurnUsageRecord {
                    input_tokens: turn.input_tokens,
                    cached_input_tokens: turn.cached_input_tokens,
                    output_tokens: turn.output_tokens,
                    model_duration_ms: turn.model_duration_ms,
                    ttft_ms: turn.ttft_ms,
                }),
            },
        );
        *last_leaf_id = Some(entry.id.clone());
        to_append.push(entry.clone());
        ctx.history_entries.push(entry);
    }
    let added = turn.input_tokens.saturating_add(turn.output_tokens);
    let store = ctx.store.clone();
    let conv_id = ctx.conversation_id.clone();
    let (today_tokens, month_tokens) = tokio::task::spawn_blocking(move || {
        for entry in &to_append {
            let _ = store.append_session_entry(&conv_id, entry);
        }
        store.record_token_usage(added)
    })
    .await
    .unwrap_or((0, 0));
    let refs: Vec<&SessionEntry> = ctx.history_entries.iter().collect();
    let breakdown =
        compute_context_breakdown(&ctx.preamble, &ctx.memory, &refs, ctx.context_window);
    let metrics = AiTurnMetricsDto {
        input_tokens: turn.input_tokens,
        cached_input_tokens: turn.cached_input_tokens,
        output_tokens: turn.output_tokens,
        model_duration_ms: turn.model_duration_ms,
        ttft_ms: turn.ttft_ms,
        today_tokens,
        month_tokens,
        context_breakdown: Some(breakdown),
    };
    let _ = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::TurnEnd {
            run_id: ctx.run_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            reasoning: turn.reasoning,
            text: turn.text,
            tokens_per_sec: turn.tokens_per_sec,
            metrics,
        })))
        .await;
}

async fn handle_tool_card(
    ctx: &mut PumpContext,
    last_leaf_id: &mut Option<String>,
    card: ToolExecutionCard,
) {
    let entry = SessionEntry::new(
        last_leaf_id.clone(),
        SessionEntryKind::ToolRecord {
            tool_name: card.tool_name.clone(),
            summary: card.summary.clone(),
            output: card.output.clone(),
            ok: card.ok,
            duration_ms: card.duration_ms,
            approved: Some(card.approved),
            diff_preview: card.diff_preview.clone(),
        },
    );
    *last_leaf_id = Some(entry.id.clone());
    let audit = AiAuditEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        conversation_id: ctx.conversation_id.clone(),
        host_label: ctx.host_label.clone(),
        tool_name: card.tool_name.clone(),
        risk: card.risk,
        summary: card.summary.clone(),
        approved: card.approved,
        ok: card.ok,
        duration_ms: card.duration_ms,
    };
    ctx.history_entries.push(entry.clone());
    {
        let store = ctx.store.clone();
        let conv_id = ctx.conversation_id.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = store.append_session_entry(&conv_id, &entry);
            let _ = store.append_audit(&audit);
        })
        .await;
    }

    let card_tool_name = card.tool_name.clone();
    let card_summary_for_hook = card.summary.clone();
    let card_host_for_hook = card.host_label.clone();
    let card_ok = card.ok;
    let card_summary_text = format!("{} ({})", card.tool_name, card.summary);

    let dto = AiToolCardDto {
        conversation_id: ctx.conversation_id.clone(),
        tool_name: card.tool_name,
        host_label: card.host_label,
        risk_label: card.risk.label().to_owned(),
        summary: card.summary,
        output: card.output,
        ok: card.ok,
        duration_ms: card.duration_ms,
        diff_preview: card.diff_preview,
    };
    let _ = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::ToolCard(dto))))
        .await;

    if let Some(exec) = &ctx.hook_executor {
        let hook_ctx = HookContext {
            tool_name: Some(card_tool_name),
            args_json: Some(card_summary_for_hook),
            target_label: card_host_for_hook,
            tool_ok: Some(card_ok),
            summary: card_summary_text,
        };
        let outcome = exec.execute(HookEvent::PostToolUse, &hook_ctx).await;
        for note in outcome.notes {
            let _ = ctx
                .event_tx
                .send_async(AppEvent::Ai(Box::new(AiAppEvent::FocusUpdated {
                    run_id: ctx.run_id.clone(),
                    focus: format!("Hook: {note}"),
                })))
                .await;
        }
    }
}
