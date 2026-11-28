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
    AiAuditEntry, AppBridgeDomain, ApprovalGrantStore, ExecPolicyRule, ExecutionMode,
    LockedSession, McpServer, McpServerBackend, McpSessionInfo, ResolvedToolTarget,
    SessionPromptContext, SessionScope, ToolOutputStore, ToolRuntimeContext, assess_tool_call,
    detect_blocking_command, format_blocking_command_hint, serve_local_socket,
};
use transom_core::{AiHostTargetDto, AppEvent};
use transom_net::{SftpClient, SshRemote};
use transom_store::AiStore;

use super::app_bridge::AppBridgeDeps;
use super::mcp_audit::{append_mcp_audit, sanitize_audit_summary};
use super::mcp_dispatch::{
    McpToolPolicyParams, dispatch_single_tool, enforce_decision, evaluate_tool_policy,
};
use crate::controller::Internal;
use crate::controller::ai_ops::control::AiApprovalMap;
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
    pub(super) exec_policy_rules: Vec<ExecPolicyRule>,
    pub(super) approval_grants: Arc<parking_lot::RwLock<ApprovalGrantStore>>,
    pub(super) command_timeout: Duration,
    pub(super) output_store: Arc<ToolOutputStore>,
    pub(super) approvals: AiApprovalMap,
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
            let meta = state.ai.mcp_session_meta.get(tab_id)?.clone();
            Some(McpLiveSession {
                meta,
                remote,
                sftp: managed.sftp_client.clone(),
            })
        })
        .collect();

    let store = state
        .ai
        .output_stores
        .get_or_create(&transom_ai::ConversationId::from("mcp-external"));
    let settings = ctx.ai_config.snapshot();
    let timeout_secs = settings.command_timeout_secs.max(5);
    let command_timeout = Duration::from_secs(timeout_secs);
    let app_bridge_deps = AppBridgeDeps::from_controller(
        state,
        ctx,
        None,
        command_timeout,
        state.ai.mcp_cancel.clone(),
    );

    if tx
        .send(McpControllerSnapshot {
            sessions,
            execution_mode: settings.execution_mode,
            exec_policy_rules: settings.exec_policy_rules.clone(),
            approval_grants: state.ai.approval_grants.clone(),
            command_timeout,
            output_store: store,
            approvals: state.ai.approvals.clone(),
            terminal_reqs: state.ai.terminal_reqs.clone(),
            event_tx: ctx.event_tx.clone(),
            ai_store: ctx.ai_store.clone(),
            app_bridge_deps,
        })
        .is_err()
    {
        log::debug!("MCP 快照请求方已取消，丢弃本次快照");
    }
}

/// 启动本地反向 MCP Socket 服务端，并将取消令牌与句柄纳入 `ControllerState`。
pub(in crate::controller) fn spawn_mcp_server(ctx: &ControllerCtx, state: &mut ControllerState) {
    let backend = Arc::new(ControllerMcpBackend {
        internal_tx: ctx.internal_tx.clone(),
        cancel: state.ai.mcp_cancel.clone(),
    });
    let server = Arc::new(McpServer::new(backend));
    let socket_path = ctx.ai_store.root().join("mcp.sock");
    let cancel = state.ai.mcp_cancel.clone();
    state.ai.mcp_task = Some(tokio::spawn(async move {
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
            return super::mcp_bridge_exec::exec_bulk(&self.cancel, &snap, &arguments).await;
        }

        if is_app_domain_tool(name) {
            return super::mcp_bridge_exec::exec_app_domain_tool(
                &self.cancel,
                name,
                &snap,
                &arguments,
            )
            .await;
        }
        if matches!(name, "remote_exec" | "remote_fs" | "transfer_file") {
            let has_target = arguments["target"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty());
            let has_session_id = arguments["session_id"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty());
            if has_target || !has_session_id || name == "transfer_file" {
                return super::mcp_bridge_exec::exec_app_domain_tool(
                    &self.cancel,
                    name,
                    &snap,
                    &arguments,
                )
                .await;
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

        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut ctx = ToolRuntimeContext::minimal(
            SessionScope::Host(sid.to_owned()),
            event_tx,
            self.cancel.clone(),
        );
        ctx.locked_session = Some(LockedSession {
            session_id: sid.to_owned(),
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
        ctx.output_store = snap.output_store.clone();
        ctx.command_timeout = snap.command_timeout;

        let args_json = arguments.to_string();
        let resolved_target = ResolvedToolTarget::new(
            Some(target.meta.session_id.to_string()),
            target.meta.host_name.clone(),
            target.meta.protection,
        );
        let assessment = assess_tool_call(name, &args_json, &ctx, &resolved_target);
        let decision = evaluate_tool_policy(McpToolPolicyParams {
            mode: snap.execution_mode,
            rules: &snap.exec_policy_rules,
            grants: &snap.approval_grants,
            tool_name: name,
            args: &arguments,
            target: &resolved_target,
            base_risk: assessment.risk,
            force_confirm: assessment.force_confirm,
        });
        let approved = enforce_decision(&snap, decision, assessment.summary, &self.cancel).await?;

        let started = Instant::now();
        let result = dispatch_single_tool(name, &arguments, &target, &snap, &self.cancel).await;
        let elapsed = started.elapsed().as_millis() as u64;
        append_mcp_audit(
            &snap,
            AiAuditEntry {
                timestamp: chrono::Utc::now().to_rfc3339(),
                conversation_id: "mcp-external".to_owned(),
                host_label: target.meta.host_name.clone(),
                tool_name: name.to_owned(),
                risk: assessment.risk,
                summary: sanitize_audit_summary(&args_json),
                approved,
                ok: result.is_ok(),
                duration_ms: elapsed,
            },
        )
        .await;
        result.map(|out| snap.output_store.store_and_preview(name, &out, 8_000))
    }
}

fn is_app_domain_tool(name: &str) -> bool {
    AppBridgeDomain::is_software_tool_name(name)
}
