//! AI 运行时事件泵、增量节流合并与 JSONL / 审计落盘。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AcpAgentConfig, AcpRunInput, AgentEvent, ChatRole, HookExecutor, MemorySnapshot, RunId,
    RunInput, SessionEntry, SessionEntryKind, run_acp_turn,
};
use transom_core::{AiAppEvent, AppEvent};

mod events;
mod persistence;
mod sink;

use super::event_pump::drive_agent_pump;
use super::run_recorder::RunRecorder;
use crate::controller::state::{AiApprovalMap, AiRunMap, HandoffSenderMap, SteerQueueMap};
use events::emit_ui_event;
use sink::MainAgentPumpSink;

/// 控制平面启动一次 AI 运行后的事件泵参数。
pub(super) struct PumpContext {
    pub(super) run_id: String,
    pub(super) conversation_id: String,
    pub(super) host_label: String,
    pub(super) model_name: String,
    pub(super) recorder: RunRecorder,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) approvals: AiApprovalMap,
    pub(super) handoffs: HandoffSenderMap,
    pub(super) runs_map: AiRunMap,
    pub(super) steers_map: SteerQueueMap,
    pub(super) title: String,
    pub(super) acp_agent: Option<AcpAgentConfig>,
    pub(super) hook_executor: Option<Arc<dyn HookExecutor>>,
    pub(super) preamble: String,
    pub(super) memory: MemorySnapshot,
    pub(super) context_window: u32,
    pub(super) history_entries: Vec<SessionEntry>,
    pub(super) current_turn_entry_id: Arc<parking_lot::RwLock<Option<String>>>,
    pub(super) cancel: CancellationToken,
}

/// 驱动 `transom_ai::run` 并同步泵送事件回 UI，同时负责 JSONL 与审计落盘。
pub(super) async fn execute_and_pump(
    input: RunInput,
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
    if let Err(err) = ctx
        .recorder
        .append_entry("追加 AI 用户提问", user_entry.clone())
        .await
    {
        finish_with_storage_error(&ctx, err.to_string()).await;
        return;
    }
    last_leaf_id = Some(user_entry.id.clone());
    *ctx.current_turn_entry_id.write() = Some(user_entry.id.clone());
    ctx.history_entries.push(user_entry.clone());
    if let Err(err) = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::RunStarted {
            run_id: ctx.run_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            title: ctx.title.clone(),
            user_entry_id: user_entry.id.clone(),
        })))
        .await
    {
        log::warn!("发送 AI 运行开始事件失败: {err}");
    }

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
            execution_mode: input.execution_mode.clone(),
            protection: input.tool_ctx.protection(),
            plan_mode: input.plan_mode,
            exec_policy_rules: input.exec_policy_rules.clone(),
            approval_scope: input.approval_scope.clone(),
            approval_grants: input.approval_grants.clone(),
            cancel: input.cancel.clone(),
        };
        tokio::spawn(async move { run_acp_turn(acp_input).await })
    } else {
        tokio::spawn(async move { transom_ai::run(input).await })
    };

    let mut sink = MainAgentPumpSink::new(ctx, last_leaf_id);
    let runner_err = drive_agent_pump(
        Some(runner_handle),
        event_rx,
        Some(Duration::from_millis(25)),
        "AI 运行任务异常中断",
        &mut sink,
    )
    .await;

    let (ctx, last_leaf_id, persistence_error) = sink.into_parts();

    let had_persistence_error = persistence_error.is_some();
    let finalize_error = ctx
        .recorder
        .finalize(last_leaf_id)
        .await
        .err()
        .map(|err| format!("更新 AI 对话活动叶节点失败: {err}"));
    let cancelled = !had_persistence_error
        && finalize_error.is_none()
        && (runner_cancel.is_cancelled()
            || runner_err
                .as_deref()
                .is_some_and(|e| e.contains("已取消") || e.contains("Cancelled")));
    let runner_err = persistence_error.or(finalize_error).or(runner_err);
    let run_id = RunId::from(ctx.run_id.as_str());
    ctx.runs_map.lock().remove(&run_id);
    ctx.steers_map.lock().remove(&run_id);
    emit_ui_event(
        &ctx,
        AiAppEvent::RunFinished {
            run_id: ctx.run_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            error: runner_err,
            cancelled,
        },
        "发送 AI 运行结束事件",
    )
    .await;
}

async fn finish_with_storage_error(ctx: &PumpContext, error: String) {
    let run_id = RunId::from(ctx.run_id.as_str());
    ctx.runs_map.lock().remove(&run_id);
    ctx.steers_map.lock().remove(&run_id);
    events::emit_ui_event(
        ctx,
        AiAppEvent::RunFinished {
            run_id: ctx.run_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            error: Some(format!("初始化 AI 会话记录失败: {error}")),
            cancelled: false,
        },
        "发送 AI 初始化失败事件",
    )
    .await;
}
