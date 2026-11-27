//! 主 Agent 有序事件消费与 UI 事件映射。

use async_trait::async_trait;
use uuid::Uuid;

use transom_ai::{AgentEvent, PlanStepStatus, ToolRisk};
use transom_core::{AiAppEvent, AiApprovalDto, AiPlanDto, AiPlanStepDto, AiPlanStepStatus};

use super::PumpContext;
use super::events::{emit_ui_event, flush_deltas};
use super::persistence::{handle_tool_card, handle_turn_end};
use crate::controller::ai_ops::event_pump::AgentPumpSink;

pub(super) struct MainAgentPumpSink {
    ctx: PumpContext,
    last_leaf_id: Option<String>,
    text_buf: String,
    reasoning_buf: String,
    persistence_error: Option<String>,
}

impl MainAgentPumpSink {
    pub(super) fn new(ctx: PumpContext, last_leaf_id: Option<String>) -> Self {
        Self {
            ctx,
            last_leaf_id,
            text_buf: String::new(),
            reasoning_buf: String::new(),
            persistence_error: None,
        }
    }

    pub(super) fn into_parts(self) -> (PumpContext, Option<String>, Option<String>) {
        (self.ctx, self.last_leaf_id, self.persistence_error)
    }
}

#[async_trait]
impl AgentPumpSink for MainAgentPumpSink {
    async fn on_tick(&mut self) {
        flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
    }

    async fn on_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta(delta) => self.text_buf.push_str(&delta),
            AgentEvent::ReasoningDelta(delta) => self.reasoning_buf.push_str(&delta),
            AgentEvent::ToolCard(card) => {
                if let Err(err) =
                    handle_tool_card(&mut self.ctx, &mut self.last_leaf_id, card).await
                {
                    self.persistence_error = Some(format!("写入 AI 工具会话记录失败: {err}"));
                    self.ctx.cancel.cancel();
                }
            }
            AgentEvent::Plan(plan) => {
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
                emit_ui_event(
                    &self.ctx,
                    AiAppEvent::PlanUpdated {
                        run_id: self.ctx.run_id.clone(),
                        conversation_id: self.ctx.conversation_id.clone(),
                        plan: dto,
                    },
                    "发送 AI 计划更新事件",
                )
                .await;
            }
            AgentEvent::FocusUpdated(focus) => {
                emit_ui_event(
                    &self.ctx,
                    AiAppEvent::FocusUpdated {
                        run_id: self.ctx.run_id.clone(),
                        focus,
                    },
                    "发送 AI 焦点更新事件",
                )
                .await;
            }
            AgentEvent::TurnEnd(turn) => {
                flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
                if let Err(err) = handle_turn_end(&mut self.ctx, &mut self.last_leaf_id, turn).await
                {
                    self.persistence_error = Some(format!("写入 AI 轮次会话记录失败: {err}"));
                    self.ctx.cancel.cancel();
                }
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
                emit_ui_event(
                    &self.ctx,
                    AiAppEvent::ApprovalRequested(dto),
                    "发送 AI 审批请求事件",
                )
                .await;
            }
            AgentEvent::TerminalHandoff(req) => {
                flush_deltas(&self.ctx, &mut self.text_buf, &mut self.reasoning_buf).await;
                self.ctx
                    .handoffs
                    .lock()
                    .insert(self.ctx.run_id.clone(), req.respond);
                emit_ui_event(
                    &self.ctx,
                    AiAppEvent::TerminalHandoffRequested {
                        run_id: self.ctx.run_id.clone(),
                        host_label: req.host_label,
                        command: req.command,
                    },
                    "发送 AI 终端接管事件",
                )
                .await;
            }
        }
    }
}
