//! AI 运行调度、快模型挑选与停机清理。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AcpAgentConfig, EVENT_CHANNEL_CAPACITY, RunInput, SessionEntry, SessionEntryKind, SessionScope,
    ThinkingEffort,
};
use transom_core::AiPromptRequest;

use super::context_builder::{
    StoreExt as _, ToolContextBuildParams, build_tool_context, emit_run_error, prepare_conversation,
};
use super::run::{PumpContext, execute_and_pump};
use super::{acp, context_builder, hooks_exec, load_and_expand_skills};
use crate::controller::state::{ControllerCtx, ControllerState};

/// 从已配置模型中挑选可用的轻量快速模型供 `model_tier = "fast"` 子 Agent 使用。
fn resolve_fast_subagent_target(
    settings: &transom_ai::AiSettings,
    primary: &transom_ai::ModelTarget,
    ctx: &ControllerCtx,
) -> Option<transom_ai::ModelTarget> {
    let fast_keywords = ["mini", "flash", "haiku", "lite", "nano", "small"];
    for (prov, model) in settings.models() {
        if model.name == primary.model && prov.base_url == primary.base_url {
            continue;
        }
        let low_id = model.id.to_ascii_lowercase();
        let low_name = model.name.to_ascii_lowercase();
        if fast_keywords
            .iter()
            .any(|k| low_id.contains(k) || low_name.contains(k))
        {
            let key = ctx
                .ai_store
                .load_provider_api_key(ctx.secret_store.as_ref(), &prov.id)
                .ok()
                .flatten()
                .map(|s| s.to_string())
                .unwrap_or_default();
            if let Ok(t) = settings.build_target(&model.id, Some(ThinkingEffort::Low), key) {
                return Some(t);
            }
        }
    }
    None
}

pub(super) async fn start_ai_run(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    mut req: AiPromptRequest,
) {
    let settings = match ctx.ai_store.load_settings() {
        Ok(s) => {
            *state.ai_execution_mode.write() = s.execution_mode;
            s
        }
        Err(err) => {
            emit_run_error(
                ctx,
                req.run_id,
                req.conversation_id.unwrap_or_default(),
                err.to_string(),
            )
            .await;
            return;
        }
    };

    let acp_agent: Option<AcpAgentConfig> = req.acp_agent_id.as_deref().and_then(|wanted| {
        settings
            .acp_agents
            .iter()
            .find(|a| a.id == wanted && a.enabled)
            .cloned()
    });

    let Some((provider, _)) = settings.resolve_model(&req.model_id) else {
        if acp_agent.is_none() {
            emit_run_error(
                ctx,
                req.run_id,
                req.conversation_id.unwrap_or_default(),
                "尚未配置任何 AI 模型，请先在设置页添加供应商与模型".to_owned(),
            )
            .await;
            return;
        }
        return acp::start_acp_only_run(state, ctx, req, acp_agent.expect("已检查存在")).await;
    };

    let api_key = match ctx
        .ai_store
        .load_provider_api_key(ctx.secret_store.as_ref(), &provider.id)
    {
        Ok(Some(secret)) => secret.to_string(),
        Ok(None) => String::new(),
        Err(err) => {
            emit_run_error(
                ctx,
                req.run_id,
                req.conversation_id.unwrap_or_default(),
                err.to_string(),
            )
            .await;
            return;
        }
    };

    let wanted_effort = req.effort.as_deref().and_then(ThinkingEffort::parse);
    let target = match settings.build_target(&req.model_id, wanted_effort, api_key) {
        Ok(t) => t,
        Err(err) => {
            emit_run_error(
                ctx,
                req.run_id,
                req.conversation_id.unwrap_or_default(),
                err.to_string(),
            )
            .await;
            return;
        }
    };

    let scope = match req.session_id {
        Some(sid) => SessionScope::Host(sid.to_string()),
        None => SessionScope::Global,
    };

    let (conversation_id, title, mut history_branch) =
        prepare_conversation(ctx, &req, &scope, &target.model).await;
    let approval_scope = transom_ai::ApprovalScope::new(
        conversation_id.clone(),
        req.session_id.map(|session_id| session_id.to_string()),
        req.protection,
    );

    let host_id_str = req.session_id.map(|s| s.to_string());
    let memory = ctx
        .ai_store
        .load_memory(host_id_str.as_deref())
        .unwrap_or_default();

    let (active_skill_prompt, effective_prompt, available_skills) =
        load_and_expand_skills(&ctx.ai_store, &req.prompt, &settings.disabled_skills);

    if let Some((label, content)) = req.attachment_text.take() {
        let parent_id = history_branch.last().map(|e| e.id.clone());
        let att = SessionEntry::new(parent_id, SessionEntryKind::Attachment { label, content });
        let _ = ctx.store_append_if_ok(&conversation_id, &att);
        history_branch.push(att);
    }

    let cancel = CancellationToken::new();
    let steer_queue = Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::new()));
    state
        .ai_runs
        .lock()
        .insert(req.run_id.clone(), cancel.clone());
    state
        .ai_steers
        .lock()
        .insert(req.run_id.clone(), steer_queue.clone());

    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let (card_tx, card_rx) = mpsc::channel(64);
    let (plan_tx, plan_rx) = mpsc::channel(32);

    let fast_target = resolve_fast_subagent_target(&settings, &target, ctx);
    let current_turn_entry_id = Arc::new(parking_lot::RwLock::new(None));
    let tool_ctx = build_tool_context(
        state,
        ctx,
        ToolContextBuildParams {
            req: &req,
            scope: &scope,
            conversation_id: &conversation_id,
            timeout_secs: settings.command_timeout_secs,
            cancel: cancel.clone(),
            card_tx,
            plan_tx: Some(plan_tx),
            subagent_cfg: Some(context_builder::SubagentConfig {
                target: &target,
                fast_target: fast_target.as_ref(),
                memory: &memory,
                response_language: Some(settings.response_language.clone()),
                exec_policy_rules: &settings.exec_policy_rules,
                hooks: &settings.hooks,
                approval_scope: &approval_scope,
                approval_grants: state.ai_approval_grants.clone(),
            }),
            current_turn_entry_id: current_turn_entry_id.clone(),
        },
    );
    let host_label = tool_ctx.host_label();
    let hook_executor: Option<Arc<dyn transom_ai::HookExecutor>> = (!settings.hooks.is_empty())
        .then(|| {
            Arc::new(hooks_exec::ControllerHookExecutor {
                rules: settings.hooks.clone(),
                remote_exec: tool_ctx.caps.remote_exec.clone(),
                cancel: cancel.clone(),
            }) as Arc<dyn transom_ai::HookExecutor>
        });
    let input = RunInput {
        target: target.clone(),
        prompt: effective_prompt,
        images_base64: req.images_base64,
        history_branch: history_branch.clone(),
        memory: memory.clone(),
        active_skill_prompt,
        available_skills,
        response_language: Some(settings.response_language.clone()),
        execution_mode: state.ai_execution_mode.clone(),
        plan_mode: req.plan_mode,
        exec_policy_rules: settings.exec_policy_rules.clone(),
        approval_scope,
        approval_grants: state.ai_approval_grants.clone(),
        hook_executor: hook_executor.clone(),
        max_turns: settings.max_turns,
        tool_ctx,
        events: event_tx,
        cancel: cancel.clone(),
        steer_queue,
    };
    let preamble_preview = transom_ai::build_run_preamble(&input, None);

    let pump_ctx = PumpContext {
        run_id: req.run_id,
        conversation_id,
        host_label,
        model_name: acp_agent
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or(target.model),
        store: ctx.ai_store.clone(),
        event_tx: ctx.event_tx.clone(),
        approvals: state.ai_approvals.clone(),
        handoffs: state.ai_handoffs.clone(),
        runs_map: state.ai_runs.clone(),
        steers_map: state.ai_steers.clone(),
        title,
        acp_agent,
        hook_executor,
        preamble: preamble_preview,
        memory,
        context_window: target.context_window,
        history_entries: history_branch,
        current_turn_entry_id,
    };

    state.ai_set.spawn(execute_and_pump(
        input, card_rx, plan_rx, event_rx, pump_ctx,
    ));
}

/// 停机时取消所有在途 AI 运行与反向 MCP 服务并限时等待回收。
pub(in crate::controller) async fn shutdown_ai_runs(state: &mut ControllerState) {
    state.mcp_cancel.cancel();
    state.ai_tasks.cancel_all();
    if let Some(task) = state.mcp_task.take() {
        let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
    }
    {
        let runs = state.ai_runs.lock();
        for token in runs.values() {
            token.cancel();
        }
    }
    state.ai_approvals.lock().clear();
    state.ai_handoffs.lock().clear();
    state.ai_terminal_reqs.lock().clear();
    state.ai_app_action_reqs.lock().clear();
    state.ai_output_stores.clear();
    state.ai_approval_grants.write().clear();
    state.ai_headless_remotes.lock().clear();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while state.ai_set.join_next().await.is_some() {}
    })
    .await;
}
