//! 内置 Agent 与 ACP 外部 Agent 共用的统一策略审批等待与授权缓存流程。
//!
//! 设计原理（Why）：
//! - `Gate::await_approval_or_handoff` 与 `acp_handlers::handle_acp_permission` 均需要计算命令前缀/影响说明、
//!   构造 `ApprovalRequest`、在 `oneshot` 通道上挂起等待用户决策并写入会话级 `ApprovalGrantStore`；
//! - 抽离为单一共享原语后，内置 Agent 与外部 ACP Agent 严格遵循同一套取消、超时与前缀授权语义。

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_ai_contracts::approval::{ApprovalGrantStore, ApprovalScope};
use transom_ai_contracts::exec_policy::{explain_command_impact, extract_command_prefix};
use transom_core::AiApprovalDecision;

use crate::runner::{AgentEvent, ApprovalRequest};
use crate::tools::ApprovalSummary;

/// 发起策略审批等待所需的共享上下文参数。
pub(crate) struct PolicyApprovalParams<'a> {
    pub(crate) events: &'a mpsc::Sender<AgentEvent>,
    pub(crate) cancel: &'a CancellationToken,
    pub(crate) summary: ApprovalSummary,
    pub(crate) command: Option<&'a str>,
    pub(crate) approval_scope: &'a ApprovalScope,
    pub(crate) approval_grants: &'a Arc<RwLock<ApprovalGrantStore>>,
    pub(crate) timeout: Option<Duration>,
}

/// 策略审批等待结果（显式区分取消、通道断开、超时与用户决策）。
pub(crate) enum PolicyApprovalOutcome {
    Decision(AiApprovalDecision),
    Cancelled,
    ChannelClosed,
    TimedOut,
}

/// 向 UI 发送审批请求并挂起等待决策；若用户选择允许同类前缀，则自动写入授权缓存。
pub(crate) async fn request_policy_approval(
    params: PolicyApprovalParams<'_>,
) -> PolicyApprovalOutcome {
    let suggested_prefix = params.command.and_then(extract_command_prefix);
    let impact_note = params.command.and_then(explain_command_impact);
    let (tx, rx) = oneshot::channel();

    if params
        .events
        .send(AgentEvent::Approval(ApprovalRequest {
            summary: params.summary,
            suggested_prefix,
            impact_note,
            respond: tx,
        }))
        .await
        .is_err()
    {
        return PolicyApprovalOutcome::ChannelClosed;
    }

    let outcome = match params.timeout {
        Some(limit) => {
            tokio::select! {
                _ = params.cancel.cancelled() => PolicyApprovalOutcome::Cancelled,
                res = tokio::time::timeout(limit, rx) => match res {
                    Ok(Ok(decision)) => PolicyApprovalOutcome::Decision(decision),
                    Ok(Err(_)) => PolicyApprovalOutcome::ChannelClosed,
                    Err(_) => PolicyApprovalOutcome::TimedOut,
                }
            }
        }
        None => {
            tokio::select! {
                _ = params.cancel.cancelled() => PolicyApprovalOutcome::Cancelled,
                res = rx => match res {
                    Ok(decision) => PolicyApprovalOutcome::Decision(decision),
                    Err(_) => PolicyApprovalOutcome::ChannelClosed,
                }
            }
        }
    };

    if let PolicyApprovalOutcome::Decision(AiApprovalDecision::AllowPrefix { ref prefix }) = outcome
        && let Some(normalized) = extract_command_prefix(prefix)
    {
        params
            .approval_grants
            .write()
            .grant_prefix(params.approval_scope, &normalized);
    }

    outcome
}
