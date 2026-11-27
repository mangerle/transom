//! 外部 MCP 单工具分发执行与策略审批模块。

use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    PolicyDecision, RemoteExecutor, RemoteFsExecutor, RuntimeInspector, TerminalContextProvider,
    ToolRisk,
};
use transom_core::{AiAppEvent, AiApprovalDto, AppEvent};
use uuid::Uuid;

use super::bridge::{HostRuntimeBridge, SshRemoteExecutor, SshRemoteFsBridge, UiTerminalBridge};
use super::mcp_bridge::{McpControllerSnapshot, McpLiveSession};

const MCP_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

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
            snap.approvals.lock().insert(request_id.clone(), tx);
            let dto = AiApprovalDto {
                request_id: request_id.clone(),
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
                snap.approvals.lock().remove(&request_id);
                return Err("UI 事件通道已关闭，无法请求人工审批".to_owned());
            }
            tokio::select! {
                _ = cancel.cancelled() => {
                    snap.approvals.lock().remove(&request_id);
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
                        snap.approvals.lock().remove(&request_id);
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
                tasks: Some(snap.app_bridge_deps.tasks.clone()),
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
            match action {
                "list" => fs.list_dir(path).await.map_err(|e| e.to_string()),
                "read" => fs
                    .read_file(path, 256 * 1024)
                    .await
                    .map_err(|e| e.to_string()),
                "search" => fs
                    .search_text(path, args["pattern"].as_str().unwrap_or(""))
                    .await
                    .map_err(|e| e.to_string()),
                "mkdir" => fs
                    .mkdir(path)
                    .await
                    .map(|()| format!("已创建目录: {path}"))
                    .map_err(|e| e.to_string()),
                "edit" => {
                    let content = args["content"].as_str().unwrap_or("");
                    fs.write_file(path, content, true)
                        .await
                        .map(|()| format!("已写入并备份: {path}"))
                        .map_err(|e| e.to_string())
                }
                "rename" => {
                    let to = args["target_path"]
                        .as_str()
                        .ok_or_else(|| "rename 必须提供 target_path".to_owned())?;
                    fs.rename(path, to)
                        .await
                        .map(|()| format!("已重命名: {path} -> {to}"))
                        .map_err(|e| e.to_string())
                }
                "delete" => {
                    let rec = args["recursive"].as_bool().unwrap_or(false);
                    fs.delete(path, rec)
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
                    fs.chmod(path, mode)
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
            insp.host_runtime_snapshot()
                .await
                .map_err(|e| e.to_string())
        }
        "container_ops" => {
            let insp = HostRuntimeBridge {
                remote: target.remote.clone(),
            };
            insp.container_action(
                args["action"].as_str().unwrap_or("list"),
                args["container_id"].as_str(),
                args["tail"].as_u64().map(|n| n as usize),
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
