//! 控制平面 AI 命令分发与生命周期管理。

mod acp;
pub(in crate::controller) mod app_bridge;
mod bridge;
mod checkpoint_ops;
mod compact;
mod context_builder;
mod hooks_exec;
mod inline_assist;
mod mcp_bridge;
mod mcp_dispatch;
mod run;
mod runtime_bridge;
mod subagent;
pub(in crate::controller) mod task_runner;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AcpAgentConfig, EVENT_CHANNEL_CAPACITY, ExecutionMode, RunInput, SessionEntry,
    SessionEntryKind, SessionScope, ThinkingEffort, extract_skill_command,
    load_skills_with_disabled, summarize_enabled_skills,
};
use transom_core::{AiAppEvent, AiCommand, AiPromptRequest, AppEvent};

use super::state::{ControllerCtx, ControllerState};
use context_builder::{StoreExt as _, build_tool_context, emit_run_error, prepare_conversation};
pub(in crate::controller) use mcp_bridge::{
    McpControllerSnapshot, handle_mcp_snapshot, spawn_mcp_server,
};
use run::{PumpContext, execute_and_pump};

/// 展开内置或自定义斜杠技能模板，返回 `(技能模板, 实际用户问题)`。
pub(super) fn expand_skill_prompt(
    store: &transom_store::AiStore,
    raw_prompt: &str,
    disabled_skills: &[String],
) -> (Option<String>, String) {
    let skills = load_skills_with_disabled(Some(&store.custom_skills_dir()), disabled_skills);
    if let Some((skill, rest)) = extract_skill_command(raw_prompt, &skills) {
        let user_text = if rest.is_empty() {
            skill.title.clone()
        } else {
            rest
        };
        (Some(skill.prompt_template.clone()), user_text)
    } else {
        (None, raw_prompt.to_owned())
    }
}

/// 处理来自 UI 的 AI 控制命令。
pub(in crate::controller) async fn handle_ai_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    cmd: AiCommand,
) {
    match cmd {
        AiCommand::SendPrompt(req) => start_ai_run(state, ctx, *req).await,
        AiCommand::CancelRun { run_id } => {
            if let Some(token) = state.ai_runs.lock().get(&run_id) {
                token.cancel();
            }
        }
        AiCommand::RespondApproval {
            request_id,
            decision,
        } => {
            if let Some(sender) = state.ai_approvals.lock().remove(&request_id) {
                let _ = sender.send(decision);
            }
        }
        AiCommand::CompactConversation {
            conversation_id,
            focus_instructions,
        } => compact::spawn_compact_conversation(
            &mut state.ai_set,
            ctx.ai_store.clone(),
            ctx.secret_store.clone(),
            ctx.event_tx.clone(),
            conversation_id,
            focus_instructions,
        ),
        AiCommand::RollbackCheckpoint {
            conversation_id,
            entry_id,
        } => checkpoint_ops::spawn_rollback_checkpoint(state, ctx, conversation_id, entry_id),
        AiCommand::InlineCommandAssist {
            request_id,
            prompt,
            host_label,
            platform,
        } => {
            inline_assist::spawn_inline_assist(state, ctx, request_id, prompt, host_label, platform)
        }
        AiCommand::ResumeHandoff {
            run_id,
            terminal_output,
            cancelled,
        } => {
            if let Some(sender) = state.ai_handoffs.lock().remove(&run_id) {
                let payload = if cancelled {
                    Err(())
                } else {
                    Ok(terminal_output)
                };
                let _ = sender.send(payload);
            }
        }
        AiCommand::RespondTerminalContext {
            request_id,
            content,
        } => {
            if let Some(sender) = state.ai_terminal_reqs.lock().remove(&request_id) {
                let _ = sender.send(content);
            }
        }
        AiCommand::RespondAppAction { request_id, result } => {
            if let Some(sender) = state.ai_app_action_reqs.lock().remove(&request_id) {
                let _ = sender.send(result);
            }
        }
        AiCommand::SetExecutionMode { mode } => {
            let parsed = match mode.as_str() {
                "safe_read_auto" => ExecutionMode::SafeReadAuto,
                "auto" => ExecutionMode::Auto,
                "allow_all" => ExecutionMode::AllowAll,
                _ => ExecutionMode::Manual,
            };
            *state.ai_execution_mode.write() = parsed;
        }
        AiCommand::DeleteConversation { conversation_id } => {
            state.ai_output_stores.remove(&conversation_id);
            if let Err(err) = ctx.ai_store.delete_conversation(&conversation_id) {
                log::warn!("删除 AI 对话失败（{conversation_id}）: {err}");
            }
        }
        AiCommand::DiscoverModels {
            request_id,
            protocol,
            base_url,
            api_key,
        } => acp::spawn_discover_models(state, ctx, request_id, protocol, base_url, api_key),
        AiCommand::TestConnection {
            request_id,
            protocol,
            base_url,
            api_key,
        } => acp::spawn_test_connection(state, ctx, request_id, protocol, base_url, api_key),
        AiCommand::InstallAcpAgent {
            agent_id,
            name,
            install_command,
        } => acp::spawn_install_acp_agent(state, ctx, agent_id, name, install_command),
        AiCommand::ProbeAcpModels { agent_id } => acp::spawn_probe_acp_models(state, ctx, agent_id),
        AiCommand::CancelManagedTask { task_id } => {
            if let Some((remote, pid)) = state.ai_tasks.cancel_task(&task_id) {
                state
                    .ai_set
                    .spawn(task_runner::kill_remote_pid(remote, pid));
            }
        }
        AiCommand::DismissManagedTask { .. } => {}
        AiCommand::DetachManagedTask { task_id } => {
            state.ai_tasks.detach_task(&task_id);
        }
        AiCommand::SteerRun { run_id, message } => {
            if let Some(queue) = state.ai_steers.lock().get(&run_id)
                && !message.trim().is_empty()
            {
                queue.lock().push_back(message);
            }
        }
    }
}

async fn start_ai_run(state: &mut ControllerState, ctx: &ControllerCtx, mut req: AiPromptRequest) {
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

    let _ = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::RunStarted {
            run_id: req.run_id.clone(),
            conversation_id: conversation_id.clone(),
            title: title.clone(),
            user_entry_id: String::new(),
        })))
        .await;

    let host_id_str = req.session_id.map(|s| s.to_string());
    let memory = ctx
        .ai_store
        .load_memory(host_id_str.as_deref())
        .unwrap_or_default();

    let (active_skill_prompt, effective_prompt) =
        expand_skill_prompt(&ctx.ai_store, &req.prompt, &settings.disabled_skills);
    let available_skills = summarize_enabled_skills(
        Some(&ctx.ai_store.custom_skills_dir()),
        &settings.disabled_skills,
    );

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

    let current_turn_entry_id = Arc::new(parking_lot::RwLock::new(None));
    let mut tool_ctx = build_tool_context(
        state,
        ctx,
        &req,
        &scope,
        &conversation_id,
        &settings.command_timeout_secs,
        cancel.clone(),
        card_tx,
        Some(plan_tx),
        Some((&target, &memory, Some(settings.response_language.clone()))),
        current_turn_entry_id.clone(),
    );
    let _ = &mut tool_ctx;
    let host_label = tool_ctx.host_label();
    let hook_executor: Option<Arc<dyn transom_ai::HookExecutor>> = (!settings.hooks.is_empty())
        .then(|| {
            Arc::new(hooks_exec::ControllerHookExecutor {
                rules: settings.hooks.clone(),
                remote_exec: tool_ctx.remote_exec.clone(),
                cancel: cancel.clone(),
            }) as Arc<dyn transom_ai::HookExecutor>
        });
    let preamble_preview = transom_ai::build_system_preamble(
        tool_ctx.locked_session.as_ref().map(|s| &s.prompt_ctx),
        &memory,
        active_skill_prompt.as_deref(),
    );

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
        session_allow_prefixes: state.ai_session_prefixes.clone(),
        hook_executor: hook_executor.clone(),
        max_turns: settings.max_turns,
        tool_ctx,
        events: event_tx,
        cancel: cancel.clone(),
        steer_queue,
    };

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
    state.ai_headless_remotes.lock().clear();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while state.ai_set.join_next().await.is_some() {}
    })
    .await;
}
