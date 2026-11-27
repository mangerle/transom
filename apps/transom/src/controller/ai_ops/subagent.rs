//! 按需子 Agent 运行器实现：在独立上下文中执行 `delegate_task` 并接入托管任务中心。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AgentEvent, DelegateMode, DelegateTaskRequest, EVENT_CHANNEL_CAPACITY, ExecPolicyRule,
    ExecutionMode, HookExecutor, HookRule, MemorySnapshot, ModelTarget, RunInput, SessionScope,
    SubagentRunner, ToolError, ToolExecutionCard, ToolOutputStore, ToolRuntimeContext,
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
    pub(in crate::controller) target: ModelTarget,
    pub(in crate::controller) memory: MemorySnapshot,
    pub(in crate::controller) response_language: Option<String>,
    pub(in crate::controller) execution_mode: Arc<RwLock<ExecutionMode>>,
    pub(in crate::controller) exec_policy_rules: Vec<ExecPolicyRule>,
    pub(in crate::controller) hooks: Vec<HookRule>,
    pub(in crate::controller) session_allow_prefixes: Arc<RwLock<HashSet<String>>>,
    pub(in crate::controller) app_bridge_deps: AppBridgeDeps,
    pub(in crate::controller) tasks: Arc<ManagedTaskRegistry>,
    pub(in crate::controller) approvals:
        Arc<Mutex<HashMap<String, oneshot::Sender<AiApprovalDecision>>>>,
    pub(in crate::controller) event_tx: flume::Sender<AppEvent>,
    pub(in crate::controller) parent_cancel: CancellationToken,
}

struct SubagentPumpSink {
    task_title: String,
    is_readonly: bool,
    task_handle: ManagedTaskHandle,
    approvals: Arc<Mutex<HashMap<String, oneshot::Sender<AiApprovalDecision>>>>,
    event_tx: flume::Sender<AppEvent>,
    final_text: String,
}

#[async_trait]
impl AgentPumpSink for SubagentPumpSink {
    async fn on_card(&mut self, card: ToolExecutionCard) {
        let status = if card.ok { "OK" } else { "ERR" };
        self.task_handle
            .emit_chunk(&format!("[{}] {} ({status})\n", card.tool_name, card.summary));
    }

    async fn on_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta(d) => {
                self.task_handle.emit_chunk(&d);
            }
            AgentEvent::TurnEnd(turn) => {
                if !turn.text.trim().is_empty() {
                    self.final_text = turn.text;
                }
            }
            AgentEvent::Approval(approval) => {
                if self.is_readonly {
                    let _ = approval.respond.send(AiApprovalDecision::RejectWithFeedback {
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
        let (card_tx, card_rx) = mpsc::channel(32);

        let tool_ctx = ToolRuntimeContext {
            scope: SessionScope::Global,
            locked_session: None,
            bulk_hosts: Vec::new(),
            output_store: Arc::new(ToolOutputStore::new()),
            command_timeout: self
                .app_bridge_deps
                .command_timeout
                .max(Duration::from_secs(30)),
            cancel: child_cancel.clone(),
            card_tx,
            plan_tx: None,
            remote_exec: None,
            remote_fs: None,
            terminal: None,
            runtime_inspector: None,
            forwarder: None,
            memory_writer: None,
            knowledge_searcher: Some(Arc::new(StoreKnowledgeBridge {
                store: self.app_bridge_deps.ai_store.clone(),
            })),
            app_bridge: Some(app_bridge),
            subagent_runner: None,
            skill_loader: Some(Arc::new(StoreSkillLoader {
                store: self.app_bridge_deps.ai_store.clone(),
            })),
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

        let input = RunInput {
            target: self.target.clone(),
            prompt,
            images_base64: Vec::new(),
            history_branch: Vec::new(),
            memory: self.memory.clone(),
            active_skill_prompt: Some(mode_instruction.to_owned()),
            available_skills: Vec::new(),
            response_language: self.response_language.clone(),
            execution_mode: exec_mode,
            plan_mode: is_readonly,
            exec_policy_rules: self.exec_policy_rules.clone(),
            session_allow_prefixes: self.session_allow_prefixes.clone(),
            hook_executor,
            max_turns: req.max_turns,
            tool_ctx,
            events: event_tx,
            cancel: child_cancel.clone(),
            steer_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        };

        let runner = tokio::spawn(async move { transom_ai::run(input).await });
        let mut sink = SubagentPumpSink {
            task_title: req.title.clone(),
            is_readonly,
            task_handle,
            approvals: self.approvals.clone(),
            event_tx: self.event_tx.clone(),
            final_text: String::new(),
        };
        let runner_err = drive_agent_pump(
            Some(runner),
            event_rx,
            card_rx,
            None,
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
        Ok(if final_text.trim().is_empty() {
            "子任务已执行完成，但未产生文本汇总。".to_owned()
        } else {
            final_text
        })
    }
}
