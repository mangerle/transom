//! 外部 MCP Server 的软件全局领域工具与多主机批量命令执行。

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AiAuditEntry, AppActionBridge, AppBridgeDomain, HostProtection, LockedSession, RemoteExecutor,
    ResolvedToolTarget, SessionPromptContext, SessionScope, ToolRuntimeContext,
    assess_shell_command, assess_tool_call, detect_blocking_command, format_blocking_command_hint,
    target_argument_for_tool,
};

use super::app_bridge::{ControllerAppActionBridge, ControllerTargetResolver};
use super::bridge::SshRemoteExecutor;
use super::mcp_audit::{append_mcp_audit, sanitize_audit_summary};
use super::mcp_bridge::McpControllerSnapshot;
use super::mcp_dispatch::{
    McpToolPolicyParams, enforce_decision, evaluate_bulk_tool_policy, evaluate_tool_policy,
};

pub(super) async fn exec_app_domain_tool(
    cancel: &CancellationToken,
    name: &str,
    snap: &McpControllerSnapshot,
    arguments: &Value,
) -> Result<String, String> {
    let matched = arguments["session_id"].as_str().and_then(|sid| {
        snap.sessions
            .iter()
            .find(|s| s.meta.session_id.to_string() == sid)
    });
    let mut deps = snap.app_bridge_deps.clone();
    if let Some(target) = matched {
        deps.target.active_tab_id = Some(target.meta.tab_id);
        deps.target.active_remote = Some(target.remote.clone());
        deps.target.active_sftp = target.sftp.clone();
    }
    let (event_tx, _event_rx) = mpsc::channel(4);
    let mut ctx = ToolRuntimeContext::minimal(SessionScope::Global, event_tx, cancel.clone());
    if let Some(target) = matched {
        ctx.locked_session = Some(LockedSession {
            session_id: target.meta.session_id.to_string(),
            prompt_ctx: SessionPromptContext {
                host_name: target.meta.host_name.clone(),
                login: target.meta.login.clone(),
                os_platform: target.meta.platform.clone(),
                protection: target.meta.protection,
            },
        });
        ctx.default_target = ResolvedToolTarget::new(
            Some(target.meta.session_id.to_string()),
            target.meta.host_name.clone(),
            target.meta.protection,
        );
    }
    ctx.output_store = snap.output_store.clone();
    ctx.command_timeout = snap.command_timeout;
    ctx.target_resolver = Some(Arc::new(ControllerTargetResolver::new(deps.clone())));
    let args_json = arguments.to_string();
    let requested_target = target_argument_for_tool(name, arguments);
    let resolved_target = ctx
        .resolve_policy_target(requested_target)
        .await
        .map_err(|error| error.to_string())?;
    let assessment = assess_tool_call(name, &args_json, &ctx, &resolved_target);
    let decision = evaluate_tool_policy(McpToolPolicyParams {
        mode: snap.execution_mode,
        rules: &snap.exec_policy_rules,
        grants: &snap.approval_grants,
        tool_name: name,
        args: arguments,
        target: &resolved_target,
        base_risk: assessment.risk,
        force_confirm: assessment.force_confirm,
    });
    let approved = enforce_decision(snap, decision, assessment.summary, cancel).await?;

    let started = Instant::now();
    let action = arguments["action"].as_str().unwrap_or("");
    let bridge = ControllerAppActionBridge::new(deps);
    let result = match AppBridgeDomain::parse(name) {
        Ok(domain) => bridge
            .invoke(domain, action, arguments.clone())
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let elapsed = started.elapsed().as_millis() as u64;
    append_mcp_audit(
        snap,
        AiAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation_id: "mcp-external".to_owned(),
            host_label: matched
                .map(|target| target.meta.host_name.clone())
                .unwrap_or_else(|| "软件全局".to_owned()),
            tool_name: name.to_owned(),
            risk: assessment.risk,
            summary: sanitize_audit_summary(&format!("[MCP] {name}({action})")),
            approved,
            ok: result.is_ok(),
            duration_ms: elapsed,
        },
    )
    .await;
    result.map(|out| snap.output_store.store_and_preview(name, &out, 8_000))
}

pub(super) async fn exec_bulk(
    cancel: &CancellationToken,
    snap: &McpControllerSnapshot,
    arguments: &Value,
) -> Result<String, String> {
    let cmd = arguments["command"]
        .as_str()
        .ok_or_else(|| "缺少 command".to_owned())?;
    if let Some(hit) = detect_blocking_command(cmd) {
        return Ok(format_blocking_command_hint(&hit));
    }
    let ids: Vec<String> = arguments["session_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut selected = Vec::new();
    for id in &ids {
        let found = snap
            .sessions
            .iter()
            .find(|s| s.meta.session_id.to_string() == *id)
            .ok_or_else(|| format!("会话 {id} 未连接或不存在"))?;
        selected.push(found.clone());
    }
    let protections: Vec<HostProtection> = selected.iter().map(|s| s.meta.protection).collect();
    let assessment = assess_shell_command(cmd);
    let (decision, requires_fresh_approval, risk) = evaluate_bulk_tool_policy(
        snap.execution_mode,
        &snap.exec_policy_rules,
        &snap.approval_grants,
        &protections,
        cmd,
        assessment.risk,
        assessment.requires_confirmation,
    );

    let host_desc = selected
        .iter()
        .map(|s| format!("{} ({})", s.meta.host_name, s.meta.login))
        .collect::<Vec<_>>()
        .join("、");
    let summary = transom_ai::ApprovalSummary {
        tool_name: "remote_exec_bulk".to_owned(),
        host_label: format!("MCP 批量 {} 台主机", selected.len()),
        protection: if protections.contains(&HostProtection::ReadOnly) {
            HostProtection::ReadOnly
        } else if protections.contains(&HostProtection::Production) {
            HostProtection::Production
        } else {
            HostProtection::Normal
        },
        risk,
        title: "外部 MCP 多主机批量执行".to_owned(),
        detail: format!("目标主机: {host_desc}\n命令: {cmd}"),
        diff_preview: None,
        requires_fresh_approval,
    };
    let approved = enforce_decision(snap, decision, summary, cancel).await?;

    let started = Instant::now();
    let mut outputs = Vec::with_capacity(selected.len());
    let mut all_ok = true;
    for item in &selected {
        let exec = SshRemoteExecutor {
            remote: item.remote.clone(),
            tasks: Some(snap.app_bridge_deps.runtime.tasks.clone()),
            host_label: item.meta.host_name.clone(),
        };
        match exec.exec(cmd, snap.command_timeout, cancel.clone()).await {
            Ok(res) => {
                if res.exit_code != 0 {
                    all_ok = false;
                }
                outputs.push(format!(
                    "=== [{} ({})] ===\n退出码: {}\n{}{}",
                    item.meta.host_name,
                    item.meta.login,
                    res.exit_code,
                    res.stdout,
                    if res.stderr.is_empty() {
                        String::new()
                    } else {
                        format!("\n[stderr] {}", res.stderr)
                    }
                ));
            }
            Err(err) => {
                all_ok = false;
                outputs.push(format!(
                    "=== [{} ({})] ===\n执行失败: {err}",
                    item.meta.host_name, item.meta.login
                ));
            }
        }
    }
    let combined = outputs.join("\n\n");
    append_mcp_audit(
        snap,
        AiAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation_id: "mcp-external".to_owned(),
            host_label: format!("批量 {} 台", selected.len()),
            tool_name: "remote_exec_bulk".to_owned(),
            risk,
            summary: sanitize_audit_summary(&format!("[MCP] {cmd}")),
            approved,
            ok: all_ok,
            duration_ms: started.elapsed().as_millis() as u64,
        },
    )
    .await;
    Ok(snap
        .output_store
        .store_and_preview("remote_exec_bulk", &combined, 8_000))
}
