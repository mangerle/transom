//! 控制平面与反向 MCP Server 之间的安全桥接。
//!
//! 设计原理（Why）：
//! - 通过有界 `internal_tx` 请求只读 [`McpControllerSnapshot`]，避免在 MCP 连接协程中直接持有控制器可变状态；
//! - 外部 MCP 客户端的每一次工具调用都必须严格经过 `detect_blocking_command`、`assess_tool_call`、
//!   `evaluate_policy` / `evaluate_bulk_policy` 与 UI 审批卡片确认，并写入 `audit.jsonl`，确保安全护栏不可绕过。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AiAuditEntry, AppActionBridge, AppBridgeDomain, ExecutionMode, HostProtection, LockedSession,
    McpServer, McpServerBackend, McpSessionInfo, RemoteExecutor, SessionPromptContext,
    SessionScope, ToolOutputStore, ToolRuntimeContext, assess_tool_call, classify_shell_command,
    detect_blocking_command, evaluate_bulk_policy, evaluate_policy, format_blocking_command_hint,
    serve_local_socket,
};
use transom_core::{AiHostTargetDto, AppEvent};
use transom_net::{SftpClient, SshRemote};
use transom_store::AiStore;

use super::app_bridge::{AppBridgeDeps, ControllerAppActionBridge};
use super::bridge::SshRemoteExecutor;
use super::mcp_dispatch::{dispatch_single_tool, enforce_decision};
use crate::controller::Internal;
use crate::controller::state::{ControllerCtx, ControllerState};

/// 单个活跃 SSH 会话在 MCP 侧的轻量执行句柄。
#[derive(Clone)]
pub(in crate::controller) struct McpLiveSession {
    pub(super) meta: AiHostTargetDto,
    pub(super) remote: SshRemote,
    pub(super) sftp: Option<SftpClient>,
}

/// 控制器向 MCP 后端提供的一次性只读快照。
pub(in crate::controller) struct McpControllerSnapshot {
    pub(super) sessions: Vec<McpLiveSession>,
    pub(super) execution_mode: ExecutionMode,
    pub(super) command_timeout: Duration,
    pub(super) output_store: Arc<ToolOutputStore>,
    pub(super) approvals:
        Arc<Mutex<HashMap<String, oneshot::Sender<transom_core::AiApprovalDecision>>>>,
    pub(super) terminal_reqs: Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) ai_store: Arc<AiStore>,
    pub(super) app_bridge_deps: AppBridgeDeps,
}

/// 响应 `Internal::McpSnapshot`，采集当前已连接的 SSH 会话与共享审批表。
pub(in crate::controller) fn handle_mcp_snapshot(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tx: oneshot::Sender<McpControllerSnapshot>,
) {
    let sessions = state
        .sessions
        .iter()
        .filter_map(|(tab_id, managed)| {
            let remote = managed.remote.clone()?;
            let meta = state.mcp_session_meta.get(tab_id)?.clone();
            Some(McpLiveSession {
                meta,
                remote,
                sftp: managed.sftp_client.clone(),
            })
        })
        .collect();

    let store = state
        .ai_output_stores
        .entry("mcp-external".to_owned())
        .or_insert_with(|| Arc::new(ToolOutputStore::new()))
        .clone();
    let timeout_secs = ctx
        .ai_store
        .load_settings()
        .map(|s| s.command_timeout_secs)
        .unwrap_or(60)
        .max(5);

    let _ = tx.send(McpControllerSnapshot {
        sessions,
        execution_mode: *state.ai_execution_mode.read(),
        command_timeout: Duration::from_secs(timeout_secs),
        output_store: store,
        approvals: state.ai_approvals.clone(),
        terminal_reqs: state.ai_terminal_reqs.clone(),
        event_tx: ctx.event_tx.clone(),
        ai_store: ctx.ai_store.clone(),
        app_bridge_deps: AppBridgeDeps {
            known_hosts: ctx.known_hosts.clone(),
            secret_store: ctx.secret_store.clone(),
            session_store: ctx.session_store.clone(),
            settings_store: ctx.settings_store.clone(),
            backup: ctx.backup.clone(),
            git_projects: ctx.git_projects.clone(),
            api_projects: ctx.api_projects.clone(),
            api_http: ctx.api_http.clone(),
            ai_store: ctx.ai_store.clone(),
            git_config: ctx.git_config.clone(),
            local_processes: ctx.local_processes.clone(),
            event_tx: ctx.event_tx.clone(),
            internal_tx: ctx.internal_tx.clone(),
            app_action_reqs: state.ai_app_action_reqs.clone(),
            active_tab_id: None,
            active_remote: None,
            active_sftp: None,
            command_timeout: Duration::from_secs(timeout_secs),
            cancel: state.mcp_cancel.clone(),
            tasks: state.ai_tasks.clone(),
            conversation_id: None,
            current_turn_entry_id: Arc::new(parking_lot::RwLock::new(None)),
            headless_remotes: state.ai_headless_remotes.clone(),
        },
    });
}

/// 启动本地反向 MCP Socket 服务端，并将取消令牌与句柄纳入 `ControllerState`。
pub(in crate::controller) fn spawn_mcp_server(ctx: &ControllerCtx, state: &mut ControllerState) {
    let backend = Arc::new(ControllerMcpBackend {
        internal_tx: ctx.internal_tx.clone(),
        cancel: state.mcp_cancel.clone(),
    });
    let server = Arc::new(McpServer::new(backend));
    let socket_path = ctx.ai_store.root().join("mcp.sock");
    let cancel = state.mcp_cancel.clone();
    state.mcp_task = Some(tokio::spawn(async move {
        if let Err(err) = serve_local_socket(server, &socket_path, cancel).await {
            log::debug!("本地 MCP Socket 未运行或已退出: {err}");
        }
    }));
}

struct ControllerMcpBackend {
    internal_tx: flume::Sender<Internal>,
    cancel: CancellationToken,
}

impl ControllerMcpBackend {
    async fn snapshot(&self) -> Result<McpControllerSnapshot, String> {
        let (tx, rx) = oneshot::channel();
        self.internal_tx
            .send_async(Internal::McpSnapshot(tx))
            .await
            .map_err(|_| "Transom 控制平面未就绪".to_owned())?;
        rx.await.map_err(|_| "无法获取会话快照".to_owned())
    }
}

#[async_trait]
impl McpServerBackend for ControllerMcpBackend {
    async fn list_sessions(&self) -> Vec<McpSessionInfo> {
        let Ok(snap) = self.snapshot().await else {
            return Vec::new();
        };
        snap.sessions
            .into_iter()
            .map(|s| McpSessionInfo {
                session_id: s.meta.session_id.to_string(),
                host_name: s.meta.host_name,
                login: s.meta.login,
                platform: s.meta.platform,
                protection: s.meta.protection,
            })
            .collect()
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        let snap = self.snapshot().await?;
        if name == "read_tool_output" {
            let output_id = arguments["output_id"]
                .as_str()
                .ok_or_else(|| "缺少 output_id".to_owned())?;
            let offset_line = arguments["offset_line"].as_u64().unwrap_or(0) as usize;
            let limit_lines = arguments["limit_lines"].as_u64().unwrap_or(0) as usize;
            let pattern = arguments["pattern"].as_str();
            return snap
                .output_store
                .read(output_id, offset_line, limit_lines, pattern)
                .map_err(|e| e.to_string());
        }

        if name == "remote_exec_bulk" {
            return self.exec_bulk(&snap, &arguments).await;
        }

        if is_app_domain_tool(name) {
            return self.exec_app_domain_tool(name, &snap, &arguments).await;
        }
        if matches!(name, "remote_exec" | "remote_fs" | "transfer_file") {
            let has_target = arguments["target"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty());
            let has_session_id = arguments["session_id"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty());
            if has_target || !has_session_id || name == "transfer_file" {
                return self.exec_app_domain_tool(name, &snap, &arguments).await;
            }
        }

        let sid = arguments["session_id"]
            .as_str()
            .ok_or_else(|| "缺少 session_id（请先调用 list_sessions 获取）".to_owned())?;
        let target = snap
            .sessions
            .iter()
            .find(|s| s.meta.session_id.to_string() == sid)
            .ok_or_else(|| format!("未找到已连接的 SSH 会话: {sid}"))?
            .clone();

        if name == "remote_exec" {
            let cmd = arguments["command"].as_str().unwrap_or("");
            if let Some(hit) = detect_blocking_command(cmd) {
                return Ok(format_blocking_command_hint(&hit));
            }
        }

        let (card_tx, _card_rx) = mpsc::channel(8);
        let ctx = ToolRuntimeContext {
            scope: SessionScope::Host(sid.to_owned()),
            locked_session: Some(LockedSession {
                session_id: sid.to_owned(),
                prompt_ctx: SessionPromptContext {
                    host_name: target.meta.host_name.clone(),
                    login: target.meta.login.clone(),
                    os_platform: target.meta.platform.clone(),
                    protection: target.meta.protection,
                },
            }),
            bulk_hosts: Vec::new(),
            output_store: snap.output_store.clone(),
            command_timeout: snap.command_timeout,
            cancel: self.cancel.clone(),
            card_tx,
            plan_tx: None,
            remote_exec: None,
            remote_fs: None,
            terminal: None,
            runtime_inspector: None,
            forwarder: None,
            memory_writer: None,
            knowledge_searcher: None,
            app_bridge: None,
            subagent_runner: None,
            skill_loader: None,
        };

        let args_json = arguments.to_string();
        let assessment = assess_tool_call(name, &args_json, &ctx);
        let decision = evaluate_policy(
            target.meta.protection,
            snap.execution_mode,
            assessment.risk,
            assessment.force_confirm,
        );
        let approved = enforce_decision(&snap, decision, assessment.summary, &self.cancel).await?;

        let started = Instant::now();
        let result = dispatch_single_tool(name, &arguments, &target, &snap, &self.cancel).await;
        let elapsed = started.elapsed().as_millis() as u64;
        let _ = snap.ai_store.append_audit(&AiAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation_id: "mcp-external".to_owned(),
            host_label: target.meta.host_name.clone(),
            tool_name: name.to_owned(),
            risk: assessment.risk,
            summary: format!("[MCP] {args_json}"),
            approved,
            ok: result.is_ok(),
            duration_ms: elapsed,
        });
        result.map(|out| snap.output_store.store_and_preview(name, &out, 8_000))
    }
}

fn is_app_domain_tool(name: &str) -> bool {
    matches!(
        name,
        "session_catalog"
            | "workspace_control"
            | "sftp_transfer"
            | "host_monitor"
            | "port_forward_ops"
            | "container_workspace"
            | "git_workspace"
            | "api_client"
            | "app_settings_ops"
            | "ai_memory_skills"
    )
}

impl ControllerMcpBackend {
    async fn exec_app_domain_tool(
        &self,
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
            deps.active_tab_id = Some(target.meta.tab_id);
            deps.active_remote = Some(target.remote.clone());
            deps.active_sftp = target.sftp.clone();
        }
        let (card_tx, _card_rx) = mpsc::channel(4);
        let ctx = ToolRuntimeContext {
            scope: SessionScope::Global,
            locked_session: None,
            bulk_hosts: Vec::new(),
            output_store: snap.output_store.clone(),
            command_timeout: snap.command_timeout,
            cancel: self.cancel.clone(),
            card_tx,
            plan_tx: None,
            remote_exec: None,
            remote_fs: None,
            terminal: None,
            runtime_inspector: None,
            forwarder: None,
            memory_writer: None,
            knowledge_searcher: None,
            app_bridge: None,
            subagent_runner: None,
            skill_loader: None,
        };
        let args_json = arguments.to_string();
        let assessment = assess_tool_call(name, &args_json, &ctx);
        let protection = matched
            .map(|t| t.meta.protection)
            .unwrap_or(HostProtection::Normal);
        let decision = evaluate_policy(
            protection,
            snap.execution_mode,
            assessment.risk,
            assessment.force_confirm,
        );
        let approved = enforce_decision(snap, decision, assessment.summary, &self.cancel).await?;

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
        let _ = snap.ai_store.append_audit(&AiAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation_id: "mcp-external".to_owned(),
            host_label: matched
                .map(|t| t.meta.host_name.clone())
                .unwrap_or_else(|| "软件全局".to_owned()),
            tool_name: name.to_owned(),
            risk: assessment.risk,
            summary: format!("[MCP] {name}({action})"),
            approved,
            ok: result.is_ok(),
            duration_ms: elapsed,
        });
        result.map(|out| snap.output_store.store_and_preview(name, &out, 8_000))
    }

    async fn exec_bulk(
        &self,
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
        let risk = classify_shell_command(cmd);
        let (decision, requires_fresh_approval) =
            evaluate_bulk_policy(&protections, snap.execution_mode, risk);

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
        let approved = enforce_decision(snap, decision, summary, &self.cancel).await?;

        let started = Instant::now();
        let mut outputs = Vec::with_capacity(selected.len());
        let mut all_ok = true;
        for item in &selected {
            let exec = SshRemoteExecutor {
                remote: item.remote.clone(),
                tasks: Some(snap.app_bridge_deps.tasks.clone()),
                host_label: item.meta.host_name.clone(),
            };
            match exec
                .exec(cmd, snap.command_timeout, self.cancel.clone())
                .await
            {
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
        let _ = snap.ai_store.append_audit(&AiAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation_id: "mcp-external".to_owned(),
            host_label: format!("批量 {} 台", selected.len()),
            tool_name: "remote_exec_bulk".to_owned(),
            risk,
            summary: format!("[MCP] {cmd}"),
            approved,
            ok: all_ok,
            duration_ms: started.elapsed().as_millis() as u64,
        });
        Ok(snap
            .output_store
            .store_and_preview("remote_exec_bulk", &combined, 8_000))
    }
}
