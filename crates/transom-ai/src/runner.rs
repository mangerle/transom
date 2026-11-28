//! 基于 `rig-core` + `rig-agent` 的流式 Agent 运行时与 `Gate` 审批门。
//!
//! 设计原理（Why）：
//! - 全程使用有界通道（`tokio::sync::mpsc::channel(256)`）向控制平面泵送事件，杜绝内存无限积压；
//! - 通过 `Gate` 实现 `AgentHook`：
//!   - `on_completion_call`：动态刷新系统提示词并执行 `CancellationToken` 与 `max_turns` 熔断；
//!   - `on_dispatch`：执行策略引擎 `evaluate_command_policy`，需要审批时挂起等待 UI `oneshot` 确认；
//!   - `on_text_delta` / `on_reasoning_delta`：记录首字时刻并流式发送增量；
//!   - `on_model_turn_finished`：计算 `tokens/s` 并发送 [`AgentEvent::TurnEnd`]，确保工具卡片与回答顺序不乱。

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};

use futures::StreamExt as _;
use parking_lot::{Mutex, RwLock};
use rig_agent::agent::{AgentBuilder, MultiTurnStreamItem};
use rig_core::driver::DynModel;
use rig_core::message::{Message, UserContent};
use rig_core::operation::Completion;
use rig_core::providers::anthropic::AnthropicConfig;
use rig_core::providers::openai::OpenAIConfig;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_core::AiApprovalDecision;

use crate::compaction::{
    PreambleExtra, SessionPromptContext, build_system_preamble_with_extra, extract_turn_pairs,
    needs_compaction,
};
use crate::hooks::{HookContext, HookEvent, HookExecutor};
use crate::runner_gate::Gate;
use crate::tools::plan::PlanSnapshot;
use crate::tools::{ApprovalSummary, ToolExecutionCard, ToolRuntimeContext, attach_tools};
use crate::working_state::{
    SessionWorkingState, aggressive_compact_history, is_context_overflow_error,
};
use transom_ai_contracts::approval::{ApprovalGrantStore, ApprovalScope};
use transom_ai_contracts::config::{ModelTarget, ProviderProtocol};
use transom_ai_contracts::error::AiError;
use transom_ai_contracts::exec_policy::ExecPolicyRule;
use transom_ai_contracts::memory::MemorySnapshot;
use transom_ai_contracts::policy::ExecutionMode;
use transom_ai_contracts::session::{ChatRole, SessionEntry};
use transom_ai_contracts::thinking::build_request_params;

/// 运行时向外泵送的事件通道容量（有界）。
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

/// 发往控制平面 / UI 的流式运行时事件。
///
/// 所有会改变会话状态的事件共用一个有序有界通道；调用方必须等待发送完成，
/// 从而保证工具卡片、计划与轮次终态不会因多通道竞争而乱序。
pub enum AgentEvent {
    TextDelta(String),
    ReasoningDelta(String),
    FocusUpdated(String),
    TurnEnd(TurnOutput),
    ToolCard(ToolExecutionCard),
    Plan(PlanSnapshot),
    Approval(ApprovalRequest),
    TerminalHandoff(HandoffRequest),
}

/// 一轮模型响应的规范化收尾结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnOutput {
    pub reasoning: String,
    pub text: String,
    pub tokens_per_sec: Option<f32>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub model_duration_ms: u64,
    pub ttft_ms: Option<u64>,
}

/// 待用户确认的审批请求。
pub struct ApprovalRequest {
    pub summary: ApprovalSummary,
    pub suggested_prefix: Option<String>,
    pub impact_note: Option<String>,
    pub respond: oneshot::Sender<AiApprovalDecision>,
}

/// 请求将高危/交互式命令移交至终端由用户手动执行的挂起通道。
pub struct HandoffRequest {
    pub host_label: String,
    pub command: String,
    pub respond: oneshot::Sender<Result<Option<String>, ()>>,
}

/// 发起一次 Agent 运行的完整输入。
pub struct RunInput {
    pub target: ModelTarget,
    pub prompt: String,
    pub images_base64: Vec<(String, String)>,
    pub history_branch: Vec<SessionEntry>,
    pub memory: MemorySnapshot,
    pub active_skill_prompt: Option<String>,
    pub available_skills: Vec<crate::skills::SkillSummary>,
    pub response_language: Option<String>,
    pub execution_mode: Arc<RwLock<ExecutionMode>>,
    pub plan_mode: bool,
    pub exec_policy_rules: Vec<ExecPolicyRule>,
    pub approval_scope: ApprovalScope,
    pub approval_grants: Arc<RwLock<ApprovalGrantStore>>,
    pub hook_executor: Option<Arc<dyn HookExecutor>>,
    pub max_turns: usize,
    pub tool_ctx: ToolRuntimeContext,
    pub events: mpsc::Sender<AgentEvent>,
    pub cancel: CancellationToken,
    pub steer_queue: Arc<Mutex<VecDeque<String>>>,
}

/// 根据 [`ModelTarget`] 构造供应商无关的 [`DynModel<Completion>`]。
pub fn build_dyn_model(target: &ModelTarget) -> DynModel<Completion> {
    match target.protocol {
        ProviderProtocol::OpenAi => {
            let mut cfg = OpenAIConfig::new(target.api_key.clone());
            if !target.base_url.trim().is_empty() {
                cfg.base_url = target.base_url.trim_end_matches('/').to_owned();
            }
            cfg.client().chat(target.model.clone()).erase()
        }
        ProviderProtocol::OpenAiResponses => {
            let mut cfg = OpenAIConfig::new(target.api_key.clone());
            if !target.base_url.trim().is_empty() {
                cfg.base_url = target.base_url.trim_end_matches('/').to_owned();
            }
            cfg.client().responses(target.model.clone()).erase()
        }
        ProviderProtocol::Anthropic => {
            let mut cfg = AnthropicConfig::new(target.api_key.clone());
            if !target.base_url.trim().is_empty() {
                cfg.base_url = target.base_url.trim_end_matches('/').to_owned();
            }
            cfg.client().completion(target.model.clone()).erase()
        }
    }
}

fn prepare_initial_history(
    input: &RunInput,
    working_state: &SessionWorkingState,
    keep_recent: usize,
    max_tool_chars: usize,
) -> (Vec<SessionEntry>, Option<String>) {
    let refs: Vec<&SessionEntry> = input.history_branch.iter().collect();
    if needs_compaction(&refs, input.target.context_window) {
        (
            aggressive_compact_history(
                &input.history_branch,
                working_state,
                keep_recent,
                max_tool_chars,
            ),
            Some(working_state.render_markdown()),
        )
    } else {
        (input.history_branch.clone(), None)
    }
}

/// 启动一次多轮流式 Agent 运行（可注入自定义/Mock [`DynModel`] 供单测使用）。
pub async fn run_with_model(model: DynModel<Completion>, input: RunInput) -> Result<(), AiError> {
    let working_state = SessionWorkingState::from_history(&input.history_branch, &input.prompt);
    let (history, ws_markdown) = prepare_initial_history(&input, &working_state, 8, 800);
    run_attempt(model, &input, &history, ws_markdown.as_deref()).await
}

/// 支持 413 / 上下文超限自动激进压缩并重试 1 次的 Agent 运行入口。
pub async fn run_with_model_factory(
    factory: impl Fn() -> DynModel<Completion>,
    input: RunInput,
) -> Result<(), AiError> {
    let working_state = SessionWorkingState::from_history(&input.history_branch, &input.prompt);
    let (initial_history, initial_ws) = prepare_initial_history(&input, &working_state, 8, 800);

    match run_attempt(factory(), &input, &initial_history, initial_ws.as_deref()).await {
        Ok(()) => Ok(()),
        Err(AiError::ModelCallFailed { ref reason, .. }) if is_context_overflow_error(reason) => {
            log::warn!(
                "检测到模型上下文超限或 413 响应，正在执行激进压缩并重注入工作状态重试: {reason}"
            );
            let compact_history =
                aggressive_compact_history(&input.history_branch, &working_state, 4, 300);
            let ws_md = working_state.render_markdown();
            run_attempt(factory(), &input, &compact_history, Some(&ws_md)).await
        }
        Err(err) => Err(err),
    }
}

/// 根据 [`RunInput`] 与可选的压缩工作状态构造完整的系统提示词（preamble）。
pub fn build_run_preamble(input: &RunInput, working_state_md: Option<&str>) -> String {
    let bulk_ctxs: Vec<SessionPromptContext> = input
        .tool_ctx
        .bulk_hosts
        .iter()
        .map(|h| SessionPromptContext {
            host_name: h.host_name.clone(),
            login: h.login.clone(),
            os_platform: "linux".to_owned(),
            protection: h.protection,
        })
        .collect();
    build_system_preamble_with_extra(
        input
            .tool_ctx
            .locked_session
            .as_ref()
            .map(|s| &s.prompt_ctx),
        &input.memory,
        input.active_skill_prompt.as_deref(),
        &PreambleExtra {
            working_state: working_state_md,
            bulk_hosts: &bulk_ctxs,
            response_language: input.response_language.as_deref(),
            available_skills: &input.available_skills,
            plan_mode: input.plan_mode,
        },
    )
}

async fn run_attempt(
    model: DynModel<Completion>,
    input: &RunInput,
    history_branch: &[SessionEntry],
    working_state_md: Option<&str>,
) -> Result<(), AiError> {
    let preamble = build_run_preamble(input, working_state_md);

    let max_turns = input.max_turns.max(1);
    let mut builder = AgentBuilder::new(model)
        .preamble(&preamble)
        .default_max_turns(max_turns);
    builder = attach_tools(builder, &input.tool_ctx);

    if let Some(max_tokens) = input.target.request_max_tokens() {
        builder = builder.max_tokens(max_tokens);
    }
    if let Some(params) = build_request_params(
        input.target.protocol,
        input.target.thinking_switch,
        input.target.effort,
        true,
    ) {
        builder = builder.additional_params(params);
    }

    let last_summary = Arc::new(Mutex::new(String::new()));
    let gate = Gate {
        events: input.events.clone(),
        tool_ctx: input.tool_ctx.clone(),
        execution_mode: input.execution_mode.clone(),
        plan_mode: input.plan_mode,
        exec_policy_rules: input.exec_policy_rules.clone(),
        approval_scope: input.approval_scope.clone(),
        approval_grants: input.approval_grants.clone(),
        hook_executor: input.hook_executor.clone(),
        max_turns,
        turns_used: AtomicUsize::new(0),
        cancel: input.cancel.clone(),
        steer_queue: input.steer_queue.clone(),
        call_started: Mutex::new(None),
        first_delta: Mutex::new(None),
        first_delta_seen: AtomicBool::new(false),
        last_summary: last_summary.clone(),
    };

    let agent = builder.add_hook(gate).build();
    let history = to_rig_history(history_branch);
    let user_msg = build_user_message(&input.prompt, &input.images_base64, input.target.vision);

    let mut stream = agent.prompt(user_msg).history(history).stream();
    let mut run_result = Ok(());
    loop {
        let next = tokio::select! {
            biased;
            _ = input.cancel.cancelled() => {
                run_result = Err(AiError::Cancelled);
                break;
            }
            item = stream.next() => item,
        };
        let Some(item) = next else {
            break;
        };
        match item {
            Ok(MultiTurnStreamItem::FinalResponse(_)) => break,
            Ok(_) => {}
            Err(err) => {
                if input.cancel.is_cancelled() {
                    run_result = Err(AiError::Cancelled);
                } else {
                    run_result = Err(AiError::ModelCallFailed {
                        protocol: input.target.protocol.as_str().to_owned(),
                        model: input.target.model.clone(),
                        reason: err.to_string(),
                    });
                }
                break;
            }
        }
    }

    if let Some(exec) = &input.hook_executor
        && !input.cancel.is_cancelled()
    {
        let summary = last_summary.lock().clone();
        let hook_ctx = HookContext {
            tool_name: None,
            args_json: None,
            command: None,
            target_label: input.tool_ctx.host_label(),
            tool_ok: Some(run_result.is_ok()),
            summary,
        };
        let outcome = exec.execute(HookEvent::Stop, &hook_ctx).await;
        if let Some(reason) = outcome.blocked_reason {
            log::warn!("AI 停止 Hook 返回失败信息: {reason}");
        }
    }

    run_result
}

/// 生产环境标准入口：根据 [`ModelTarget`] 构造真实客户端并执行（支持 413 自动降级重试）。
pub async fn run(input: RunInput) -> Result<(), AiError> {
    let target = input.target.clone();
    run_with_model_factory(move || build_dyn_model(&target), input).await
}

async fn run_single_turn_text(
    target: &ModelTarget,
    preamble: &str,
    prompt: &str,
    timeout_secs: u64,
    operation: &str,
) -> Result<String, AiError> {
    use rig_agent::agent::{AgentHook, HookContext, ModelTurnAction, ModelTurnFinished};
    use rig_core::message::AssistantContent;

    struct TextCollector(Arc<Mutex<String>>);
    impl AgentHook for TextCollector {
        async fn on_model_turn_finished(
            &self,
            _ctx: &HookContext,
            event: ModelTurnFinished<'_>,
        ) -> ModelTurnAction {
            let mut buf = self.0.lock();
            for item in event.content {
                if let AssistantContent::Text(t) = item {
                    buf.push_str(&t.text);
                }
            }
            ModelTurnAction::continue_run()
        }
    }

    let model = build_dyn_model(target);
    let collected = Arc::new(Mutex::new(String::new()));
    let agent = AgentBuilder::new(model)
        .preamble(preamble)
        .add_hook(TextCollector(collected.clone()))
        .build();
    let fut = async {
        let mut stream = agent.prompt(Message::user(prompt.to_owned())).stream();
        while let Some(item) = stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::FinalResponse(_)) => break,
                Ok(_) => {}
                Err(err) => {
                    return Err(AiError::ModelCallFailed {
                        protocol: target.protocol.as_str().to_owned(),
                        model: target.model.clone(),
                        reason: err.to_string(),
                    });
                }
            }
        }
        Ok(collected.lock().clone())
    };
    tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut)
        .await
        .map_err(|_| AiError::Timeout {
            operation: operation.to_owned(),
            timeout_secs,
        })?
}

/// 单轮无工具轻量生成终端内联命令（Ctrl/Cmd+I），带 20 秒超时。
///
/// # Errors
/// 模型调用失败或 20 秒超时时返回 [`AiError`]。
pub async fn generate_inline_command(
    target: &ModelTarget,
    prompt: &str,
    host_label: &str,
    platform: &str,
) -> Result<String, AiError> {
    let preamble = format!(
        "你是终端命令行助手。目标主机: {host_label}，平台: {platform}。\n\
        请把用户的自然语言需求转换为一条可直接执行的单行 Shell 命令。\n\
        必须严格只输出如下 JSON（不要 markdown 代码块，不要多余文字）：\n\
        {{\"command\":\"...\",\"explanation\":\"一句话中文解释\"}}"
    );
    run_single_turn_text(target, &preamble, prompt, 20, "内联命令生成").await
}

/// 调用当前模型生成保留指定重点的上下文压缩摘要（`/compact`）。
///
/// # Errors
/// 模型调用失败、返回空摘要或 25 秒超时时返回 [`AiError`]。
pub async fn summarize_for_compaction(
    target: &ModelTarget,
    transcript: &str,
    focus: &str,
) -> Result<String, AiError> {
    let focus_line = if focus.trim().is_empty() {
        "请提炼已确认的事实、关键路径/端口/配置变更、已完成事项与未解决的问题。".to_owned()
    } else {
        format!(
            "请务必重点保留以下内容：{}\n同时保留关键主机、路径、配置变更与未完成事项。",
            focus.trim()
        )
    };
    let preamble = format!(
        "你是运维对话上下文压缩器。请将以下长对话记录压缩为精炼、结构化的中文摘要。\n{focus_line}\n只输出摘要正文，不要寒暄。"
    );
    let raw = run_single_turn_text(target, &preamble, transcript, 25, "上下文压缩摘要").await?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AiError::ModelCallFailed {
            protocol: target.protocol.as_str().to_owned(),
            model: target.model.clone(),
            reason: "模型返回空摘要".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

fn to_rig_history(entries: &[SessionEntry]) -> Vec<Message> {
    let refs: Vec<&SessionEntry> = entries.iter().collect();
    let pairs = extract_turn_pairs(&refs);
    let mut merged: Vec<(ChatRole, String)> = Vec::with_capacity(pairs.len());
    for (role, text) in pairs {
        if let Some((last_role, last_text)) = merged.last_mut()
            && *last_role == role
        {
            last_text.push_str("\n\n");
            last_text.push_str(&text);
        } else {
            merged.push((role, text));
        }
    }
    merged
        .into_iter()
        .map(|(role, text)| match role {
            ChatRole::User => Message::user(text),
            ChatRole::Assistant => Message::assistant(text),
        })
        .collect()
}

fn build_user_message(prompt: &str, images_base64: &[(String, String)], vision: bool) -> Message {
    if !vision || images_base64.is_empty() {
        return Message::user(prompt.to_owned());
    }
    let mut contents = Vec::with_capacity(images_base64.len() + 1);
    contents.push(UserContent::text(prompt.to_owned()));
    for (media_type, b64) in images_base64 {
        let media = match media_type.as_str() {
            "image/jpeg" => Some(rig_core::message::ImageMediaType::JPEG),
            "image/webp" => Some(rig_core::message::ImageMediaType::WEBP),
            "image/gif" => Some(rig_core::message::ImageMediaType::GIF),
            _ => Some(rig_core::message::ImageMediaType::PNG),
        };
        contents.push(UserContent::image_base64(b64.clone(), media, None));
    }
    Message::User { content: contents }
}
