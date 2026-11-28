//! 控制平面 ACP 外部 Agent 独立调度与模型探测辅助。

use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::RunId;
use transom_ai::{
    AcpAgentConfig, EVENT_CHANNEL_CAPACITY, ProviderProtocol, RunInput, SessionScope,
    default_acp_presets, discover_models, probe_acp_models, test_provider_connection,
};
use transom_core::{AiAppEvent, AiPromptRequest};

use super::context_builder::{ToolContextBuildParams, emit_run_error};
use super::credentials::load_provider_api_key;
use super::process::{MAX_PROCESS_CAPTURE_BYTES, capture_command_output};
use super::run::{PumpContext, execute_and_pump};
use super::run_recorder::RunRecorder;
use super::{build_tool_context, prepare_conversation};
use crate::controller::state::{ControllerCtx, ControllerState};

pub(super) fn parse_protocol(raw: &str) -> ProviderProtocol {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" => ProviderProtocol::Anthropic,
        "responses" => ProviderProtocol::OpenAiResponses,
        _ => ProviderProtocol::OpenAi,
    }
}

/// 供应商连通性与模型发现参数。
pub(super) struct ConnectionProbeParams {
    pub(super) request_id: String,
    pub(super) provider_id: Option<String>,
    pub(super) protocol: String,
    pub(super) base_url: String,
    pub(super) api_key: String,
}

async fn resolve_probe_api_key(
    store: std::sync::Arc<transom_store::AiStore>,
    secrets: std::sync::Arc<dyn transom_core::SecretStore>,
    typed: String,
    provider_id: Option<String>,
) -> Result<String, String> {
    if !typed.trim().is_empty() {
        return Ok(typed);
    }
    let Some(provider_id) = provider_id else {
        return Ok(String::new());
    };
    load_provider_api_key(store, secrets, provider_id)
        .await
        .map(|value| value.unwrap_or_default())
}

pub(super) fn spawn_discover_models(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    params: ConnectionProbeParams,
) {
    let proto = parse_protocol(&params.protocol);
    let event_tx = ctx.event_tx.clone();
    let store = ctx.ai_store.clone();
    let secrets = ctx.secret_store.clone();
    state.ai.run_set.spawn(async move {
        let api_key =
            match resolve_probe_api_key(store, secrets, params.api_key, params.provider_id).await {
                Ok(api_key) => api_key,
                Err(error) => {
                    super::send_ai_event(
                        &event_tx,
                        AiAppEvent::ModelsDiscovered {
                            request_id: params.request_id,
                            models: Vec::new(),
                            error: Some(error),
                        },
                        "模型发现凭据读取失败",
                    )
                    .await;
                    return;
                }
            };
        let event = match discover_models(proto, &params.base_url, &api_key).await {
            Ok(models) => AiAppEvent::ModelsDiscovered {
                request_id: params.request_id,
                models: models
                    .into_iter()
                    .map(|model| (model.id, model.context_window))
                    .collect(),
                error: None,
            },
            Err(error) => AiAppEvent::ModelsDiscovered {
                request_id: params.request_id,
                models: Vec::new(),
                error: Some(error.to_string()),
            },
        };
        super::send_ai_event(&event_tx, event, "模型发现结果").await;
    });
}

pub(super) fn spawn_test_connection(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    params: ConnectionProbeParams,
) {
    let proto = parse_protocol(&params.protocol);
    let event_tx = ctx.event_tx.clone();
    let store = ctx.ai_store.clone();
    let secrets = ctx.secret_store.clone();
    state.ai.run_set.spawn(async move {
        let api_key =
            match resolve_probe_api_key(store, secrets, params.api_key, params.provider_id).await {
                Ok(api_key) => api_key,
                Err(error) => {
                    super::send_ai_event(
                        &event_tx,
                        AiAppEvent::ConnectionTested {
                            request_id: params.request_id,
                            ok: false,
                            detail: error,
                        },
                        "连接测试凭据读取失败",
                    )
                    .await;
                    return;
                }
            };
        let event = match test_provider_connection(proto, &params.base_url, &api_key).await {
            Ok(result) => AiAppEvent::ConnectionTested {
                request_id: params.request_id,
                ok: true,
                detail: result.detail,
            },
            Err(error) => AiAppEvent::ConnectionTested {
                request_id: params.request_id,
                ok: false,
                detail: error.to_string(),
            },
        };
        super::send_ai_event(&event_tx, event, "供应商连通性测试结果").await;
    });
}

pub(super) async fn start_acp_only_run(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    req: AiPromptRequest,
    agent: AcpAgentConfig,
) {
    let scope = match req.session_id {
        Some(sid) => SessionScope::Host(sid.to_string()),
        None => SessionScope::Global,
    };
    let (conversation_id, title, history_branch) =
        match prepare_conversation(ctx, &req, &scope, &agent.name).await {
            Ok(conversation) => conversation,
            Err(err) => {
                emit_run_error(
                    ctx,
                    req.run_id,
                    req.conversation_id.unwrap_or_default(),
                    format!("准备 ACP 对话失败: {err}"),
                )
                .await;
                return;
            }
        };
    let approval_scope = transom_ai::ApprovalScope::new(
        conversation_id.clone(),
        req.session_id.map(|session_id| session_id.to_string()),
        req.protection,
    );
    let recorder = RunRecorder::new(ctx.ai_store.clone(), conversation_id.clone());

    let cancel = CancellationToken::new();
    state
        .ai
        .runs
        .lock()
        .insert(RunId::from(req.run_id.as_str()), cancel.clone());
    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let tool_ctx = build_tool_context(
        state,
        ctx,
        ToolContextBuildParams {
            req: &req,
            scope: &scope,
            conversation_id: &conversation_id,
            timeout_secs: 60,
            cancel: cancel.clone(),
            events: event_tx.clone(),
            subagent_cfg: None,
            current_turn_entry_id: Arc::new(parking_lot::RwLock::new(None)),
        },
    );
    let host_label = tool_ctx.host_label();
    let effective_model = req
        .model_id
        .strip_prefix("acp:")
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| agent.default_model.clone());
    let settings = ctx.ai_config.snapshot();
    let execution_mode = Arc::new(parking_lot::RwLock::new(settings.execution_mode));
    let (active_skill_prompt, mut effective_prompt, _) =
        super::load_and_expand_skills(&ctx.ai_store, &req.prompt, &settings.disabled_skills);
    if let Some(skill_body) = active_skill_prompt.as_deref() {
        effective_prompt = if effective_prompt.trim().is_empty() {
            skill_body.to_owned()
        } else {
            format!("{skill_body}\n\n{effective_prompt}")
        };
    }
    if let Some((label, content)) = req.attachment_text {
        effective_prompt.push_str(&format!("\n\n[附件: {label}]\n{content}"));
    }
    let input = RunInput {
        target: transom_ai::ModelTarget {
            protocol: ProviderProtocol::OpenAi,
            base_url: String::new(),
            api_key: String::new(),
            model: effective_model.clone(),
            vision: false,
            effort: None,
            thinking_switch: Default::default(),
            max_tokens: None,
            context_window: 128_000,
        },
        prompt: effective_prompt,
        images_base64: Vec::new(),
        history_branch: history_branch.clone(),
        memory: Default::default(),
        active_skill_prompt,
        available_skills: Vec::new(),
        response_language: Some(settings.response_language),
        execution_mode,
        plan_mode: req.plan_mode,
        exec_policy_rules: settings.exec_policy_rules.clone(),
        approval_scope,
        approval_grants: state.ai.approval_grants.clone(),
        hook_executor: None,
        max_turns: 15,
        tool_ctx,
        events: event_tx,
        cancel: cancel.clone(),
        steer_queue: Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::new())),
    };
    let pump_ctx = PumpContext {
        run_id: req.run_id,
        conversation_id,
        host_label,
        model_name: if effective_model.is_empty() {
            agent.name.clone()
        } else {
            effective_model
        },
        recorder,
        event_tx: ctx.event_tx.clone(),
        approvals: state.ai.approvals.clone(),
        handoffs: state.ai.handoffs.clone(),
        runs_map: state.ai.runs.clone(),
        steers_map: state.ai.steers.clone(),
        title,
        acp_agent: Some(agent),
        hook_executor: None,
        budget_tracker: {
            let refs: Vec<&transom_ai::SessionEntry> = history_branch.iter().collect();
            transom_ai::ContextBudgetTracker::new(
                "",
                &transom_ai::MemorySnapshot::default(),
                &refs,
                0,
                128_000,
            )
        },
        current_turn_entry_id: Arc::new(parking_lot::RwLock::new(None)),
        cancel,
    };
    state
        .ai
        .run_set
        .spawn(execute_and_pump(input, event_rx, pump_ctx));
}

pub(super) fn spawn_install_acp_agent(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    agent_id: String,
    name: String,
    install_command: String,
) {
    let event_tx = ctx.event_tx.clone();
    state.ai.run_set.spawn(async move {
        super::send_ai_event(
            &event_tx,
            AiAppEvent::AcpInstallStatus {
                agent_id: agent_id.clone(),
                running: true,
                ok: None,
                detail: format!("正在安装 {name}：{install_command}"),
            },
            "ACP 安装开始",
        )
        .await;

        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", &install_command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-lc", &install_command]);
            c
        };
        let result = match capture_command_output(
            &mut cmd,
            "acp_install",
            Duration::from_secs(300),
            CancellationToken::new(),
            MAX_PROCESS_CAPTURE_BYTES,
        )
        .await
        {
            Ok(output) => {
                let ok = output.status.success();
                let mut combined = format!("{}{}", output.stdout_lossy(), output.stderr_lossy());
                if output.stdout_truncated || output.stderr_truncated {
                    combined.push_str("\n[安装输出超过上限，已截断]");
                }
                let tail: String = combined
                    .lines()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" | ");
                let summary: String = tail.chars().take(400).collect();
                let detail = if ok {
                    format!("{name} 安装完成")
                } else if summary.is_empty() {
                    format!("{name} 安装失败（退出码 {:?}）", output.status.code())
                } else {
                    format!("{name} 安装失败：{summary}")
                };
                (ok, detail)
            }
            Err(error) => (false, format!("{name} 安装进程失败: {error}")),
        };

        super::send_ai_event(
            &event_tx,
            AiAppEvent::AcpInstallStatus {
                agent_id,
                running: false,
                ok: Some(result.0),
                detail: result.1,
            },
            "ACP 安装完成",
        )
        .await;
    });
}

pub(super) fn spawn_probe_acp_models(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    agent_id: String,
) {
    let settings = ctx.ai_config.snapshot();
    let Some(agent) = settings
        .acp_agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .or_else(|| default_acp_presets().into_iter().find(|a| a.id == agent_id))
    else {
        return;
    };
    let event_tx = ctx.event_tx.clone();
    state.ai.run_set.spawn(async move {
        match probe_acp_models(&agent).await {
            Ok(res) => {
                super::send_ai_event(
                    &event_tx,
                    AiAppEvent::AcpModelsProbed {
                        agent_id,
                        current_model: res.current_model,
                        models: res.models,
                    },
                    "ACP 模型探测完成",
                )
                .await;
            }
            Err(err) => {
                log::debug!("探测 ACP Agent（{}）模型列表失败: {err}", agent.id);
            }
        }
    });
}
