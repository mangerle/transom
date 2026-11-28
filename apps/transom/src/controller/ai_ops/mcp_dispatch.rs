//! 外部 MCP 单工具分发执行与策略审批模块。

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    ApprovalGrantStore, ApprovalRequestId, ApprovalScope, CommandPolicyContext, ExecPolicyRule,
    ExecutionMode, HostProtection, PolicyDecision, RemoteExecutor, RemoteFsExecutor,
    ResolvedToolTarget, RuntimeInspector, TerminalContextProvider, ToolIoContext, ToolRisk,
    evaluate_bulk_policy, evaluate_command_chain, evaluate_command_policy, evaluate_policy,
};
use transom_core::{AiAppEvent, AiApprovalDto, AppEvent};
use uuid::Uuid;

use super::bridge::{HostRuntimeBridge, SshRemoteExecutor, SshRemoteFsBridge, UiTerminalBridge};
use super::mcp_bridge::{McpControllerSnapshot, McpLiveSession};

const MCP_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const MCP_APPROVAL_NAMESPACE: &str = "mcp-external";

/// 外部 MCP 单工具策略评估参数。
pub(super) struct McpToolPolicyParams<'a> {
    pub(super) mode: ExecutionMode,
    pub(super) rules: &'a [ExecPolicyRule],
    pub(super) grants: &'a Arc<RwLock<ApprovalGrantStore>>,
    pub(super) tool_name: &'a str,
    pub(super) args: &'a Value,
    pub(super) target: &'a ResolvedToolTarget,
    pub(super) base_risk: ToolRisk,
    pub(super) force_confirm: bool,
}

/// 计算外部 MCP 单工具调用的统一策略裁决。
///
/// 外部 MCP 使用独立授权命名空间，但必须与内置 Agent 一样应用用户命令规则、
/// 命令链风险和审批授权，避免外部客户端获得更宽的自动执行权限。
pub(super) fn evaluate_tool_policy(params: McpToolPolicyParams<'_>) -> PolicyDecision {
    let Some(command) = (params.tool_name == "remote_exec")
        .then(|| params.args["command"].as_str())
        .flatten()
    else {
        return evaluate_policy(
            params.target.protection,
            params.mode,
            params.base_risk,
            params.force_confirm,
        );
    };
    let scope = ApprovalScope::new(
        MCP_APPROVAL_NAMESPACE,
        params.target.host_id.clone(),
        params.target.protection,
    );
    let grants = params.grants.read();
    evaluate_command_policy(&CommandPolicyContext {
        protection: params.target.protection,
        mode: params.mode,
        plan_mode: false,
        base_risk: params.base_risk,
        force_confirm: params.force_confirm,
        command: Some(command),
        exec_policy_rules: params.rules,
        approval_scope: &scope,
        approval_grants: &grants,
    })
}

/// 计算外部 MCP 多主机批量执行的统一策略裁决。
///
/// 先执行与内置 Agent 相同的命令链规则，再应用多主机保护策略；最终风险取两者
/// 的较高等级，任何强制确认信号都会阻止自动放行。
pub(super) fn evaluate_bulk_tool_policy(
    mode: ExecutionMode,
    rules: &[ExecPolicyRule],
    grants: &Arc<RwLock<ApprovalGrantStore>>,
    protections: &[HostProtection],
    command: &str,
    base_risk: ToolRisk,
    requires_confirmation: bool,
) -> (PolicyDecision, bool, ToolRisk) {
    let worst_protection = if protections.contains(&HostProtection::ReadOnly) {
        HostProtection::ReadOnly
    } else if protections.contains(&HostProtection::Production) {
        HostProtection::Production
    } else {
        HostProtection::Normal
    };
    let scope = ApprovalScope::new(MCP_APPROVAL_NAMESPACE, None, worst_protection);
    let grants = grants.read();
    let chain = evaluate_command_chain(command, rules, &grants, &scope);
    if let Some(reason) = chain.denied_reason {
        return (PolicyDecision::Deny { reason }, false, chain.worst_risk);
    }
    let risk = base_risk.max(chain.worst_risk);
    let (mut decision, mut requires_fresh_approval) = evaluate_bulk_policy(protections, mode, risk);
    if decision == PolicyDecision::Allow
        && (requires_confirmation || chain.requires_confirmation || chain.force_ask)
    {
        decision = PolicyDecision::RequireApproval;
        requires_fresh_approval = true;
    }
    (decision, requires_fresh_approval, risk)
}

pub(super) async fn enforce_decision(
    snap: &McpControllerSnapshot,
    decision: PolicyDecision,
    summary: transom_ai::ApprovalSummary,
    cancel: &CancellationToken,
) -> Result<bool, String> {
    match decision {
        PolicyDecision::Allow => Ok(true),
        PolicyDecision::Deny { reason } => Err(reason),
        PolicyDecision::RequireApproval => {
            let request_id = Uuid::new_v4().to_string();
            let (tx, rx) = oneshot::channel();
            snap.approvals
                .lock()
                .insert(ApprovalRequestId::from(request_id.as_str()), tx);
            let dto = AiApprovalDto {
                request_id: request_id.clone(),
                owner_run_id: "mcp-external".to_owned(),
                conversation_id: None,
                run_id: "mcp-external".to_owned(),
                tool_name: summary.tool_name,
                host_label: summary.host_label,
                protection: summary.protection,
                risk_label: summary.risk.label().to_owned(),
                is_destructive: summary.risk == ToolRisk::Destructive,
                title: summary.title,
                detail: summary.detail,
                diff_preview: summary.diff_preview,
                requires_fresh_approval: summary.requires_fresh_approval,
                suggested_prefix: None,
                impact_note: None,
            };
            if snap
                .event_tx
                .send_async(AppEvent::Ai(Box::new(AiAppEvent::ApprovalRequested(dto))))
                .await
                .is_err()
            {
                snap.approvals
                    .lock()
                    .remove(&ApprovalRequestId::from(request_id.as_str()));
                return Err("UI 事件通道已关闭，无法请求人工审批".to_owned());
            }
            tokio::select! {
                _ = cancel.cancelled() => {
                    snap.approvals
                        .lock()
                        .remove(&ApprovalRequestId::from(request_id.as_str()));
                    Err("Transom 正在退出，审批已取消".to_owned())
                }
                res = tokio::time::timeout(MCP_APPROVAL_TIMEOUT, rx) => match res {
                    Ok(Ok(
                        transom_core::AiApprovalDecision::AllowOnce
                        | transom_core::AiApprovalDecision::AllowPrefix { .. },
                    )) => Ok(true),
                    Ok(Ok(_)) => Err("用户拒绝了来自外部 MCP 的操作请求".to_owned()),
                    Ok(Err(_)) => Err("审批通道已断开".to_owned()),
                    Err(_) => {
                        snap.approvals
                            .lock()
                            .remove(&ApprovalRequestId::from(request_id.as_str()));
                        Err("外部 MCP 审批等待超时（120 秒），用户未响应，已自动拒绝".to_owned())
                    }
                }
            }
        }
    }
}

pub(super) async fn dispatch_single_tool(
    name: &str,
    args: &Value,
    target: &McpLiveSession,
    snap: &McpControllerSnapshot,
    cancel: &CancellationToken,
) -> Result<String, String> {
    match name {
        "remote_exec" => {
            let cmd = args["command"].as_str().unwrap_or("");
            let exec = SshRemoteExecutor {
                remote: target.remote.clone(),
                tasks: Some(snap.app_bridge_deps.runtime.tasks.clone()),
                host_label: target.meta.host_name.clone(),
            };
            let out = exec
                .exec(cmd, snap.command_timeout, cancel.clone())
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "退出码: {}\n{}{}",
                out.exit_code,
                out.stdout,
                if out.stderr.is_empty() {
                    String::new()
                } else {
                    format!("\n[stderr]\n{}", out.stderr)
                }
            ))
        }
        "remote_fs" => {
            let fs = SshRemoteFsBridge {
                remote: target.remote.clone(),
                sftp: target.sftp.clone(),
            };
            let action = args["action"].as_str().unwrap_or("read");
            let path = args["path"].as_str().unwrap_or("");
            let io = ToolIoContext::new(snap.command_timeout, cancel.clone());
            match action {
                "list" => fs
                    .list_dir_with_io(path, &io)
                    .await
                    .map_err(|e| e.to_string()),
                "read" => fs
                    .read_file_with_io(path, 256 * 1024, &io)
                    .await
                    .map_err(|e| e.to_string()),
                "search" => fs
                    .search_text_with_io(path, args["pattern"].as_str().unwrap_or(""), &io)
                    .await
                    .map_err(|e| e.to_string()),
                "mkdir" => fs
                    .mkdir_with_io(path, &io)
                    .await
                    .map(|()| format!("已创建目录: {path}"))
                    .map_err(|e| e.to_string()),
                "edit" => {
                    let content = args["content"].as_str().unwrap_or("");
                    fs.write_file_with_io(path, content, true, &io)
                        .await
                        .map(|()| format!("已写入并备份: {path}"))
                        .map_err(|e| e.to_string())
                }
                "rename" => {
                    let to = args["target_path"]
                        .as_str()
                        .ok_or_else(|| "rename 必须提供 target_path".to_owned())?;
                    fs.rename_with_io(path, to, &io)
                        .await
                        .map(|()| format!("已重命名: {path} -> {to}"))
                        .map_err(|e| e.to_string())
                }
                "delete" => {
                    let rec = args["recursive"].as_bool().unwrap_or(false);
                    fs.delete_with_io(path, rec, &io)
                        .await
                        .map(|()| format!("已删除: {path}"))
                        .map_err(|e| e.to_string())
                }
                "chmod" => {
                    let raw = args["mode"]
                        .as_str()
                        .ok_or_else(|| "chmod 必须提供八进制 mode".to_owned())?;
                    let mode = u32::from_str_radix(raw.trim_start_matches('0'), 8)
                        .map_err(|_| format!("无效权限: {raw}"))?;
                    fs.chmod_with_io(path, mode, &io)
                        .await
                        .map(|()| format!("已修改权限 {raw}: {path}"))
                        .map_err(|e| e.to_string())
                }
                other => Err(format!("未知 remote_fs action: {other}")),
            }
        }
        "inspect_host_runtime" => {
            let insp = HostRuntimeBridge {
                remote: target.remote.clone(),
            };
            insp.host_runtime_snapshot_with_io(&ToolIoContext::new(
                snap.command_timeout,
                cancel.clone(),
            ))
            .await
            .map_err(|e| e.to_string())
        }
        "container_ops" => {
            let insp = HostRuntimeBridge {
                remote: target.remote.clone(),
            };
            let io = ToolIoContext::new(snap.command_timeout, cancel.clone());
            insp.container_action_with_io(
                args["action"].as_str().unwrap_or("list"),
                args["container_id"].as_str(),
                args["tail"].as_u64().map(|n| n as usize),
                &io,
            )
            .await
            .map_err(|e| e.to_string())
        }
        "get_recent_blocks" => {
            let term = UiTerminalBridge {
                tab_id: target.meta.tab_id,
                event_tx: snap.event_tx.clone(),
                pending: snap.terminal_reqs.clone(),
            };
            let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, 20) as usize;
            term.recent_blocks(limit).await.map_err(|e| e.to_string())
        }
        "read_terminal" => {
            let term = UiTerminalBridge {
                tab_id: target.meta.tab_id,
                event_tx: snap.event_tx.clone(),
                pending: snap.terminal_reqs.clone(),
            };
            let lines = args["lines"].as_u64().unwrap_or(80).clamp(10, 300) as usize;
            term.read_screen_tail(lines)
                .await
                .map_err(|e| e.to_string())
        }
        other => Err(format!("不支持的 MCP 工具: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::RwLock;
    use serde_json::json;

    use super::{McpToolPolicyParams, evaluate_tool_policy};
    use transom_ai::{
        ApprovalGrantStore, ExecPolicyAction, ExecPolicyRule, ExecutionMode, HostProtection,
        PolicyDecision, ResolvedToolTarget, ToolRisk,
    };

    fn target() -> ResolvedToolTarget {
        ResolvedToolTarget::new(
            Some("prod-01".to_owned()),
            "prod-01",
            HostProtection::Production,
        )
    }

    fn deny_rule(pattern: &str) -> ExecPolicyRule {
        ExecPolicyRule {
            id: "deny".to_owned(),
            pattern: pattern.to_owned(),
            action: ExecPolicyAction::Deny,
            note: "禁止外部 MCP 自动执行".to_owned(),
            enabled: true,
        }
    }

    #[test]
    fn mcp_remote_exec_respects_user_deny_rules() {
        let grants = Arc::new(RwLock::new(ApprovalGrantStore::new()));
        let args = json!({ "command": "systemctl restart nginx" });
        let decision = evaluate_tool_policy(McpToolPolicyParams {
            mode: ExecutionMode::AllowAll,
            rules: &[deny_rule("systemctl restart")],
            grants: &grants,
            tool_name: "remote_exec",
            args: &args,
            target: &target(),
            base_risk: ToolRisk::Write,
            force_confirm: false,
        });
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn mcp_remote_exec_ask_rule_overrides_allow_all() {
        let grants = Arc::new(RwLock::new(ApprovalGrantStore::new()));
        let rule = ExecPolicyRule {
            id: "ask".to_owned(),
            pattern: "docker restart".to_owned(),
            action: ExecPolicyAction::Ask,
            note: String::new(),
            enabled: true,
        };
        let args = json!({ "command": "docker restart api" });
        let decision = evaluate_tool_policy(McpToolPolicyParams {
            mode: ExecutionMode::AllowAll,
            rules: &[rule],
            grants: &grants,
            tool_name: "remote_exec",
            args: &args,
            target: &target(),
            base_risk: ToolRisk::Write,
            force_confirm: false,
        });
        assert_eq!(decision, PolicyDecision::RequireApproval);
    }
}
