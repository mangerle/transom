//! 按需子 Agent 运行器实现：在独立上下文中执行 `delegate_task` 并接入托管任务中心。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AgentEvent, ChatRole, DelegateMode, DelegateTaskRequest, EVENT_CHANNEL_CAPACITY,
    ExecPolicyRule, ExecutionMode, HookExecutor, HookRule, MemorySnapshot, ModelTarget, RunInput,
    SessionEntry, SessionEntryKind, SessionScope, SubagentModelTier, SubagentRunner, ToolError,
    ToolRuntimeContext,
};
use transom_core::{AiAppEvent, AiApprovalDecision, AiApprovalDto, AiManagedTaskStatus, AppEvent};
use uuid::Uuid;

use super::app_bridge::{
    AppBridgeDeps, ControllerAppActionBridge, StoreKnowledgeBridge, StoreSkillLoader,
};
use super::event_pump::{AgentPumpSink, drive_agent_pump};
use super::hooks_exec::ControllerHookExecutor;
use super::task_runner::{ManagedTaskHandle, ManagedTaskRegistry};

pub(in crate::controller) struct ControllerSubagentRunner {
    pub(in crate::controller) parent_run_id: String,
    pub(in crate::controller) target: ModelTarget,
    pub(in crate::controller) fast_target: Option<ModelTarget>,
    pub(in crate::controller) memory: MemorySnapshot,
    pub(in crate::controller) response_language: Option<String>,
    pub(in crate::controller) execution_mode: Arc<RwLock<ExecutionMode>>,
    pub(in crate::controller) exec_policy_rules: Vec<ExecPolicyRule>,
    pub(in crate::controller) hooks: Vec<HookRule>,
    pub(in crate::controller) approval_scope: transom_ai::ApprovalScope,
    pub(in crate::controller) approval_grants: Arc<RwLock<transom_ai::ApprovalGrantStore>>,
    pub(in crate::controller) app_bridge_deps: AppBridgeDeps,
    pub(in crate::controller) tasks: Arc<ManagedTaskRegistry>,
    pub(in crate::controller) subagent_histories: Arc<Mutex<HashMap<String, Vec<SessionEntry>>>>,
    pub(in crate::controller) approvals:
        Arc<Mutex<HashMap<String, oneshot::Sender<AiApprovalDecision>>>>,
    pub(in crate::controller) event_tx: flume::Sender<AppEvent>,
    pub(in crate::controller) parent_cancel: CancellationToken,
}

struct SubagentPumpSink {
    task_title: String,
    parent_run_id: String,
    conversation_id: Option<String>,
    is_readonly: bool,
    task_handle: ManagedTaskHandle,
    approvals: Arc<Mutex<HashMap<String, oneshot::Sender<AiApprovalDecision>>>>,
    event_tx: flume::Sender<AppEvent>,
    final_text: String,
}

#[async_trait]
impl AgentPumpSink for SubagentPumpSink {
    async fn on_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta(delta) => {
                self.task_handle.emit_chunk(&delta);
            }
            AgentEvent::ToolCard(card) => {
                let status = if card.ok { "OK" } else { "ERR" };
                self.task_handle.emit_chunk(&format!(
                    "[{}] {} ({status})\n",
                    card.tool_name, card.summary
                ));
            }
            AgentEvent::Plan(_) | AgentEvent::FocusUpdated(_) | AgentEvent::TerminalHandoff(_) => {}
            AgentEvent::TurnEnd(turn) => {
                if !turn.text.trim().is_empty() {
                    self.final_text = turn.text;
                }
            }
            AgentEvent::Approval(approval) => {
                if self.is_readonly {
                    let _ = approval
                        .respond
                        .send(AiApprovalDecision::RejectWithFeedback {
                            reason: "只读子 Agent 不允许写操作".to_owned(),
                        });
                    self.task_handle
                        .emit_chunk("[已拦截只读子 Agent 的写操作请求]\n");
                } else {
                    let request_id = Uuid::new_v4().to_string();
                    self.approvals
                        .lock()
                        .insert(request_id.clone(), approval.respond);
                    let dto = AiApprovalDto {
                        request_id,
                        owner_run_id: self.parent_run_id.clone(),
                        conversation_id: self.conversation_id.clone(),
                        run_id: self.task_handle.task_id.clone(),
                        tool_name: approval.summary.tool_name,
                        host_label: approval.summary.host_label,
                        protection: approval.summary.protection,
                        risk_label: approval.summary.risk.label().to_owned(),
                        is_destructive: approval.summary.risk == transom_ai::ToolRisk::Destructive,
                        title: format!("[子任务: {}] {}", self.task_title, approval.summary.title),
                        detail: approval.summary.detail,
                        diff_preview: approval.summary.diff_preview,
                        requires_fresh_approval: approval.summary.requires_fresh_approval,
                        suggested_prefix: approval.suggested_prefix,
                        impact_note: approval.impact_note,
                    };
                    let _ = self
                        .event_tx
                        .send_async(AppEvent::Ai(Box::new(AiAppEvent::ApprovalRequested(dto))))
                        .await;
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl SubagentRunner for ControllerSubagentRunner {
    async fn run_subagent(&self, req: DelegateTaskRequest) -> Result<String, ToolError> {
        let target_label = req
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("全局")
            .to_owned();
        let task_handle = self.tasks.start_task(
            "subagent",
            &req.title,
            &target_label,
            &self.parent_cancel,
            None,
        );
        let child_cancel = task_handle.cancel.clone();

        let mut deps = self.app_bridge_deps.clone();
        deps.cancel = child_cancel.clone();
        let app_bridge: Arc<dyn transom_ai::AppActionBridge> =
            Arc::new(ControllerAppActionBridge::new(deps));

        let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

        let mut tool_ctx = ToolRuntimeContext::minimal(
            SessionScope::Global,
            event_tx.clone(),
            child_cancel.clone(),
        );
        tool_ctx.command_timeout = self
            .app_bridge_deps
            .command_timeout
            .max(Duration::from_secs(30));
        tool_ctx.caps = transom_ai::ToolCapabilities {
            knowledge_searcher: Some(Arc::new(StoreKnowledgeBridge {
                store: self.app_bridge_deps.ai_store.clone(),
            })),
            app_bridge: Some(app_bridge),
            skill_loader: Some(Arc::new(StoreSkillLoader {
                store: self.app_bridge_deps.ai_store.clone(),
            })),
            ..Default::default()
        };

        let is_readonly = req.mode == DelegateMode::Readonly;
        let exec_mode = if is_readonly {
            Arc::new(RwLock::new(ExecutionMode::SafeReadAuto))
        } else {
            self.execution_mode.clone()
        };
        let mode_instruction = if is_readonly {
            "你是主运维助手派发的【只读调研子 Agent】。严禁执行任何写入、修改、创建、删除或重启操作；仅使用只读工具收集证据并给出精炼结论。"
        } else {
            "你是主运维助手派发的【独立部署子 Agent】。请高效完成分配的子任务并返回明确的验证结果摘要。"
        };
        let prompt = match &req.target {
            Some(t) if !t.trim().is_empty() => {
                format!("默认目标: {}\n\n任务要求：\n{}", t.trim(), req.task)
            }
            _ => req.task.clone(),
        };

        let hook_executor: Option<Arc<dyn HookExecutor>> = (!self.hooks.is_empty()).then(|| {
            Arc::new(ControllerHookExecutor {
                rules: self.hooks.clone(),
                remote_exec: None,
                cancel: child_cancel.clone(),
            }) as Arc<dyn HookExecutor>
        });

        let steer_queue = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        self.tasks
            .attach_steer_queue(&task_handle.task_id, steer_queue.clone());

        let subagent_id = req
            .resume_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| task_handle.task_id.clone());
        let history_branch = self
            .subagent_histories
            .lock()
            .get(&subagent_id)
            .cloned()
            .unwrap_or_default();

        let chosen_target = match req.model_tier {
            SubagentModelTier::Fast => self
                .fast_target
                .clone()
                .unwrap_or_else(|| self.target.clone()),
            SubagentModelTier::Inherit => self.target.clone(),
        };
        let used_model = chosen_target.model.clone();

        let input = RunInput {
            target: chosen_target,
            prompt: prompt.clone(),
            images_base64: Vec::new(),
            history_branch: history_branch.clone(),
            memory: self.memory.clone(),
            active_skill_prompt: Some(mode_instruction.to_owned()),
            available_skills: Vec::new(),
            response_language: self.response_language.clone(),
            execution_mode: exec_mode,
            plan_mode: is_readonly,
            exec_policy_rules: self.exec_policy_rules.clone(),
            approval_scope: self.approval_scope.child(&task_handle.task_id),
            approval_grants: self.approval_grants.clone(),
            hook_executor,
            max_turns: req.max_turns,
            tool_ctx,
            events: event_tx,
            cancel: child_cancel.clone(),
            steer_queue,
        };

        let mut sink = SubagentPumpSink {
            task_title: req.title.clone(),
            parent_run_id: self.parent_run_id.clone(),
            conversation_id: self.app_bridge_deps.conversation_id.clone(),
            is_readonly,
            task_handle,
            approvals: self.approvals.clone(),
            event_tx: self.event_tx.clone(),
            final_text: String::new(),
        };
        let subagent_histories = self.subagent_histories.clone();
        let conv_id = self
            .app_bridge_deps
            .conversation_id
            .clone()
            .unwrap_or_default();
        let notify_tx = self.event_tx.clone();
        let task_title = req.title;
        let async_mode = req.async_mode;

        let exec_fut = async move {
            let runner = tokio::spawn(async move { transom_ai::run(input).await });
            let runner_err = drive_agent_pump(
                Some(runner),
                event_rx,
                None,
                "子 Agent 任务异常中断",
                &mut sink,
            )
            .await;
            let SubagentPumpSink {
                task_handle,
                final_text,
                ..
            } = sink;

            if child_cancel.is_cancelled() {
                task_handle.finish(AiManagedTaskStatus::Cancelled, None, "子任务已取消");
                return Err(ToolError::Cancelled);
            }
            if let Some(err) = runner_err {
                task_handle.finish(AiManagedTaskStatus::Failed, None, &err);
                return Err(ToolError::ExecutionFailed {
                    tool: "delegate_task",
                    reason: err,
                });
            }
            task_handle.finish(AiManagedTaskStatus::Succeeded, Some(0), "子任务完成");

            let reply_body = if final_text.trim().is_empty() {
                "子任务已执行完成，但未产生文本汇总。".to_owned()
            } else {
                final_text
            };

            save_subagent_history(
                &subagent_histories,
                &subagent_id,
                history_branch,
                prompt,
                reply_body.clone(),
                used_model,
            );

            Ok((subagent_id, reply_body))
        };

        if async_mode {
            tokio::spawn(async move {
                let res = exec_fut.await;
                let (ok, sid, summary) = match res {
                    Ok((id, text)) => (true, id, text),
                    Err(err) => (false, String::new(), err.to_string()),
                };
                if !conv_id.is_empty() {
                    let _ = notify_tx
                        .send_async(AppEvent::Ai(Box::new(AiAppEvent::SubagentCompleted {
                            conversation_id: conv_id,
                            subagent_id: sid,
                            title: task_title,
                            ok,
                            summary,
                        })))
                        .await;
                }
            });
            return Ok(
                "已将该子任务转入后台异步执行（可在底部任务栏实时查看或干预）。完成后将自动把结果回传并唤醒主对话继续汇总。"
                    .to_owned(),
            );
        }

        let (sid, reply_body) = exec_fut.await?;
        Ok(format!("[subagent_id: {sid}]\n{reply_body}"))
    }
}

fn save_subagent_history(
    cache_lock: &Mutex<HashMap<String, Vec<SessionEntry>>>,
    subagent_id: &str,
    mut history_branch: Vec<SessionEntry>,
    prompt: String,
    reply_body: String,
    used_model: String,
) {
    let parent_u = history_branch.last().map(|e| e.id.clone());
    let user_entry = SessionEntry::new(
        parent_u,
        SessionEntryKind::Message {
            role: ChatRole::User,
            text: prompt,
            images: Vec::new(),
            model: None,
            tokens_per_sec: None,
            usage: None,
        },
    );
    let assistant_entry = SessionEntry::new(
        Some(user_entry.id.clone()),
        SessionEntryKind::Message {
            role: ChatRole::Assistant,
            text: reply_body,
            images: Vec::new(),
            model: Some(used_model),
            tokens_per_sec: None,
            usage: None,
        },
    );
    history_branch.push(user_entry);
    history_branch.push(assistant_entry);

    let mut cache = cache_lock.lock();
    if cache.len() >= 32
        && !cache.contains_key(subagent_id)
        && let Some(oldest_key) = cache.keys().next().cloned()
    {
        cache.remove(&oldest_key);
    }
    cache.insert(subagent_id.to_owned(), history_branch);
}
