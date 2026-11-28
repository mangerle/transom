//! AI 运行调度、快模型挑选与停机清理。

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::RunId;
use transom_ai::{
    AcpAgentConfig, EVENT_CHANNEL_CAPACITY, RunInput, SessionEntry, SessionEntryKind, SessionScope,
    ThinkingEffort,
};
use transom_core::AiPromptRequest;

use super::context_builder::{
    ToolContextBuildParams, build_tool_context, emit_run_error, prepare_conversation,
};
use super::run::{PumpContext, execute_and_pump};
use super::run_recorder::RunRecorder;
use super::{
    acp, context_builder, credentials::load_provider_api_key, hooks_exec, load_and_expand_skills,
};
use crate::controller::state::{ControllerCtx, ControllerState};

/// 从已配置模型中挑选可用的轻量快速模型供 `model_tier = "fast"` 子 Agent 使用。
async fn resolve_fast_subagent_target(
    settings: &transom_ai::AiSettings,
    primary: &transom_ai::ModelTarget,
    ctx: &ControllerCtx,
) -> Option<transom_ai::ModelTarget> {
    let settings = settings.clone();
    let primary = primary.clone();
    let store = ctx.ai_store.clone();
    let secrets = ctx.secret_store.clone();
    tokio::task::spawn_blocking(move || {
        let fast_keywords = ["mini", "flash", "haiku", "lite", "nano", "small"];
        for (prov, model) in settings.models() {
            if model.name == primary.model && prov.base_url == primary.base_url {
                continue;
            }
            let low_id = model.id.to_ascii_lowercase();
            let low_name = model.name.to_ascii_lowercase();
            if fast_keywords
                .iter()
                .any(|keyword| low_id.contains(keyword) || low_name.contains(keyword))
            {
                let key = store
                    .load_provider_api_key(secrets.as_ref(), &prov.id)
                    .ok()
                    .flatten()
                    .map(|secret| secret.to_string())
                    .unwrap_or_default();
                if let Ok(target) = settings.build_target(&model.id, Some(ThinkingEffort::Low), key)
                {
                    return Some(target);
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// 主 Agent 与 ACP 独立运行共用的基础运行环境准备结果。
pub(super) struct PreparedRunBase {
    pub(super) scope: SessionScope,
    pub(super) conversation_id: String,
    pub(super) title: String,
    pub(super) history_branch: Vec<SessionEntry>,
    pub(super) approval_scope: transom_ai::ApprovalScope,
    pub(super) recorder: RunRecorder,
    pub(super) memory: transom_ai::MemorySnapshot,
    pub(super) active_skill_prompt: Option<String>,
    pub(super) effective_prompt: String,
    pub(super) available_skills: Vec<transom_ai::SkillSummary>,
    pub(super) cancel: CancellationToken,
    pub(super) steer_queue: Arc<parking_lot::Mutex<std::collections::VecDeque<String>>>,
    pub(super) event_tx: mpsc::Sender<transom_ai::AgentEvent>,
    pub(super) event_rx: mpsc::Receiver<transom_ai::AgentEvent>,
    pub(super) current_turn_entry_id: Arc<parking_lot::RwLock<Option<String>>>,
}

/// 准备主 Agent 与 ACP 独立运行共用的会话分支、记忆/技能上下文与运行注册状态。
pub(super) async fn prepare_run_base(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    req: &AiPromptRequest,
    model_label: &str,
    disabled_skills: &[String],
    load_memory: bool,
) -> Option<PreparedRunBase> {
    let scope = match req.session_id {
        Some(sid) => SessionScope::Host(sid.to_string()),
        None => SessionScope::Global,
    };
    let (conversation_id, title, history_branch) =
        match prepare_conversation(ctx, req, &scope, model_label).await {
            Ok(conversation) => conversation,
            Err(err) => {
                emit_run_error(
                    ctx,
                    req.run_id.clone(),
                    req.conversation_id.clone().unwrap_or_default(),
                    format!("准备 AI 对话失败: {err}"),
                )
                .await;
                return None;
            }
        };
    let approval_scope = transom_ai::ApprovalScope::new(
        conversation_id.clone(),
        req.session_id.map(|session_id| session_id.to_string()),
        req.protection,
    );
    let recorder = RunRecorder::new(ctx.ai_store.clone(), conversation_id.clone());

    let host_id_str = req.session_id.map(|session_id| session_id.to_string());
    let prompt_for_skills = req.prompt.clone();
    let disabled = disabled_skills.to_vec();
    let store = ctx.ai_store.clone();
    let prompt_context = tokio::task::spawn_blocking(move || {
        let memory = if load_memory {
            store.load_memory(host_id_str.as_deref())?
        } else {
            transom_ai::MemorySnapshot::default()
        };
        let (active_skill_prompt, effective_prompt, available_skills) =
            load_and_expand_skills(&store, &prompt_for_skills, &disabled);
        Ok::<_, transom_ai::AiError>((
            memory,
            active_skill_prompt,
            effective_prompt,
            available_skills,
        ))
    })
    .await;
    let (memory, active_skill_prompt, effective_prompt, available_skills) = match prompt_context {
        Ok(Ok(context)) => context,
        Ok(Err(err)) => {
            emit_run_error(
                ctx,
                req.run_id.clone(),
                conversation_id,
                format!("加载 AI 记忆或技能失败: {err}"),
            )
            .await;
            return None;
        }
        Err(err) => {
            emit_run_error(
                ctx,
                req.run_id.clone(),
                conversation_id,
                format!("AI 启动准备任务异常: {err}"),
            )
            .await;
            return None;
        }
    };

    let cancel = CancellationToken::new();
    let steer_queue = Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::new()));
    let run_key = RunId::from(req.run_id.as_str());
    state.ai.runs.lock().insert(run_key.clone(), cancel.clone());
    state.ai.steers.lock().insert(run_key, steer_queue.clone());

    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let current_turn_entry_id = Arc::new(parking_lot::RwLock::new(None));

    Some(PreparedRunBase {
        scope,
        conversation_id,
        title,
        history_branch,
        approval_scope,
        recorder,
        memory,
        active_skill_prompt,
        effective_prompt,
        available_skills,
        cancel,
        steer_queue,
        event_tx,
        event_rx,
        current_turn_entry_id,
    })
}

pub(super) async fn start_ai_run(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    mut req: AiPromptRequest,
) {
    let settings = ctx.ai_config.snapshot();
    let execution_mode = Arc::new(parking_lot::RwLock::new(settings.execution_mode));

    let acp_agent: Option<AcpAgentConfig> = req.acp_agent_id.as_deref().and_then(|wanted| {
        settings
            .acp_agents
            .iter()
            .find(|a| a.id == wanted && a.enabled)
            .cloned()
    });

    let Some((provider, _)) = settings.resolve_model(&req.model_id) else {
        if let Some(agent) = acp_agent {
            return acp::start_acp_only_run(state, ctx, req, agent).await;
        }
        emit_run_error(
            ctx,
            req.run_id,
            req.conversation_id.unwrap_or_default(),
            "尚未配置任何 AI 模型，请先在设置页添加供应商与模型".to_owned(),
        )
        .await;
        return;
    };

    let api_key = match load_provider_api_key(
        ctx.ai_store.clone(),
        ctx.secret_store.clone(),
        provider.id.clone(),
    )
    .await
    {
        Ok(Some(secret)) => secret,
        Ok(None) => String::new(),
        Err(err) => {
            emit_run_error(
                ctx,
                req.run_id,
                req.conversation_id.unwrap_or_default(),
                err,
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

    let Some(base) = prepare_run_base(
        state,
        ctx,
        &req,
        &target.model,
        &settings.disabled_skills,
        true,
    )
    .await
    else {
        return;
    };
    let PreparedRunBase {
        scope,
        conversation_id,
        title,
        mut history_branch,
        approval_scope,
        recorder,
        memory,
        active_skill_prompt,
        effective_prompt,
        available_skills,
        cancel,
        steer_queue,
        event_tx,
        event_rx,
        current_turn_entry_id,
    } = base;

    if let Some((label, content)) = req.attachment_text.take() {
        let parent_id = history_branch.last().map(|e| e.id.clone());
        let attachment =
            SessionEntry::new(parent_id, SessionEntryKind::Attachment { label, content });
        if let Err(err) = recorder
            .append_entry("追加 AI 附件上下文", attachment.clone())
            .await
        {
            emit_run_error(
                ctx,
                req.run_id,
                conversation_id,
                format!("保存 AI 附件上下文失败: {err}"),
            )
            .await;
            return;
        }
        history_branch.push(attachment);
    }

    let fast_target = resolve_fast_subagent_target(&settings, &target, ctx).await;
    let tool_ctx = build_tool_context(
        state,
        ctx,
        ToolContextBuildParams {
            req: &req,
            scope: &scope,
            conversation_id: &conversation_id,
            timeout_secs: settings.command_timeout_secs,
            cancel: cancel.clone(),
            events: event_tx.clone(),
            subagent_cfg: Some(context_builder::SubagentConfig {
                parent_run_id: &req.run_id,
                target: &target,
                fast_target: fast_target.as_ref(),
                memory: &memory,
                response_language: Some(settings.response_language.clone()),
                exec_policy_rules: &settings.exec_policy_rules,
                hooks: &settings.hooks,
                approval_scope: &approval_scope,
                approval_grants: state.ai.approval_grants.clone(),
                execution_mode: execution_mode.clone(),
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
        execution_mode,
        plan_mode: req.plan_mode,
        exec_policy_rules: settings.exec_policy_rules.clone(),
        approval_scope,
        approval_grants: state.ai.approval_grants.clone(),
        hook_executor: hook_executor.clone(),
        max_turns: settings.max_turns,
        tool_ctx,
        events: event_tx,
        cancel: cancel.clone(),
        steer_queue,
    };
    let preamble_preview = transom_ai::build_run_preamble(&input, None);
    let tools_tokens = if acp_agent.is_some() {
        0
    } else {
        transom_ai::estimate_tool_schema_tokens(&input.tool_ctx)
    };

    let history_refs: Vec<&SessionEntry> = history_branch.iter().collect();
    let budget_tracker = transom_ai::ContextBudgetTracker::new(
        &preamble_preview,
        &memory,
        &history_refs,
        tools_tokens,
        target.context_window,
    );

    let pump_ctx = PumpContext {
        run_id: req.run_id,
        conversation_id,
        host_label,
        model_name: acp_agent
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or(target.model),
        recorder: recorder.clone(),
        event_tx: ctx.event_tx.clone(),
        approvals: state.ai.approvals.clone(),
        handoffs: state.ai.handoffs.clone(),
        runs_map: state.ai.runs.clone(),
        steers_map: state.ai.steers.clone(),
        title,
        acp_agent,
        hook_executor,
        budget_tracker,
        current_turn_entry_id,
        cancel,
    };

    state
        .ai
        .run_set
        .spawn(execute_and_pump(input, event_rx, pump_ctx));
}

/// 停机时取消所有在途 AI 运行与反向 MCP 服务并限时等待回收。
pub(in crate::controller) async fn shutdown_ai_runs(state: &mut ControllerState) {
    state.ai.shutdown().await;
}
