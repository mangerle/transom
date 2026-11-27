//! 软件领域全局工具（第 1~5 领域：会话目录、工作区标签/面板、SFTP 同步、主机监控、端口转发）。

use std::time::Instant;

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppBridgeDomain, ToolExecutionCard, ToolRuntimeContext, to_rig_err};
use crate::error::ToolError;
use crate::policy::ToolRisk;

#[derive(Deserialize)]
pub struct AppActionArgs {
    pub action: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// 统一委托 [`AppActionBridge`] 执行软件操作并记录工具卡片。
pub(crate) async fn invoke_app_tool(
    ctx: &ToolRuntimeContext,
    tool_name: &'static str,
    risk: ToolRisk,
    args: AppActionArgs,
) -> Result<String, ToolExecutionError> {
    let bridge = ctx.app_bridge.as_ref().ok_or_else(|| {
        to_rig_err(ToolError::ExecutionFailed {
            tool: tool_name,
            reason: "当前运行上下文未挂载软件操作桥接器".to_owned(),
        })
    })?;
    let domain = AppBridgeDomain::parse(tool_name).map_err(to_rig_err)?;
    let started = Instant::now();
    let payload = Value::Object(args.extra);
    let summary = format!("{tool_name}({})", args.action);
    let res = bridge.invoke(domain, &args.action, payload).await;
    let elapsed = started.elapsed().as_millis() as u64;
    match res {
        Ok(raw) => {
            let clipped = ctx.format_tool_output(tool_name, &raw);
            ctx.record_card(ToolExecutionCard {
                tool_name: tool_name.to_owned(),
                host_label: String::new(),
                risk,
                summary,
                output: clipped.clone(),
                ok: true,
                approved: true,
                duration_ms: elapsed,
                diff_preview: None,
            });
            Ok(clipped)
        }
        Err(err) => {
            let msg = err.to_string();
            ctx.record_card(ToolExecutionCard {
                tool_name: tool_name.to_owned(),
                host_label: String::new(),
                risk,
                summary,
                output: msg,
                ok: false,
                approved: true,
                duration_ms: elapsed,
                diff_preview: None,
            });
            Err(to_rig_err(err))
        }
    }
}

/// 领域 1：会话与目录管理（增删改查、分组、known_hosts、部署公钥）。
pub struct SessionCatalogTool {
    ctx: ToolRuntimeContext,
}

impl SessionCatalogTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for SessionCatalogTool {
    const NAME: &'static str = "session_catalog";
    type Error = ToolExecutionError;
    type Args = AppActionArgs;
    type Output = String;

    fn description(&self) -> String {
        "管理 Transom 已保存会话目录与受信任主机：list / create / update / delete / \
         create_folder / delete_folder / list_known_hosts / remove_known_host / deploy_public_key。"
            .to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list", "create", "update", "delete",
                        "create_folder", "delete_folder",
                        "list_known_hosts", "remove_known_host", "deploy_public_key"
                    ]
                },
                "session_id": { "type": "string", "description": "目标会话 ID" },
                "name": { "type": "string", "description": "会话或目录名称" },
                "kind": { "type": "string", "enum": ["ssh", "local", "rdp", "serial"], "description": "会话类型，默认 ssh" },
                "host": { "type": "string", "description": "主机地址或域名" },
                "port": { "type": "integer", "description": "端口，默认 22" },
                "username": { "type": "string", "description": "登录用户名" },
                "private_key_path": { "type": "string", "description": "可选私钥路径" },
                "folder_id": { "type": "string", "description": "所属文件夹 ID" },
                "parent_id": { "type": "string", "description": "父文件夹 ID" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "protection": { "type": "string", "enum": ["normal", "production", "read_only"] },
                "jump_session_id": { "type": "string", "description": "跳板机会话 ID" },
                "public_key_path": { "type": "string", "description": "deploy_public_key 时的公钥路径" }
            },
            "required": ["action"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let risk = if matches!(
            args.action.as_str(),
            "delete" | "delete_folder" | "remove_known_host" | "deploy_public_key"
        ) {
            ToolRisk::Destructive
        } else if matches!(args.action.as_str(), "list" | "list_known_hosts") {
            ToolRisk::SafeRead
        } else {
            ToolRisk::Write
        };
        invoke_app_tool(&self.ctx, Self::NAME, risk, args).await
    }
}

/// 领域 2：终端与工作区控制（标签页、打开本地终端、连接会话、分屏、切换面板、发送输入）。
pub struct WorkspaceControlTool {
    ctx: ToolRuntimeContext,
}

impl WorkspaceControlTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for WorkspaceControlTool {
    const NAME: &'static str = "workspace_control";
    type Error = ToolExecutionError;
    type Args = AppActionArgs;
    type Output = String;

    fn description(&self) -> String {
        "控制 Transom 主窗口标签页与面板：list_tabs / connect_session / open_local_terminal / \
         switch_tab / close_tab / split_pane / open_panel / send_terminal_input。\
         注意：仅在用户明确要求打开窗口/终端或必须交互操作时才调用 connect_session；\
         常规远端巡检、命令与文件传输请直接通过执行工具的 target 参数在后台静默连接，不要打开标签页。"
            .to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list_tabs", "connect_session", "open_local_terminal",
                        "switch_tab", "close_tab", "split_pane", "open_panel", "send_terminal_input"
                    ]
                },
                "session_id": { "type": "string", "description": "connect_session 时的已保存会话 ID" },
                "name": { "type": "string", "description": "按会话名称匹配连接" },
                "tab_id": { "type": "string", "description": "目标标签页 ID" },
                "cwd": { "type": "string", "description": "open_local_terminal 初始目录" },
                "axis": { "type": "string", "enum": ["horizontal", "vertical"], "description": "split_pane 方向" },
                "panel": {
                    "type": "string",
                    "enum": ["sessions", "sftp", "monitor", "forwards", "containers", "git", "api", "ai", "snippets", "sync"]
                },
                "input": { "type": "string", "description": "send_terminal_input 发送的文本（末尾可含 \\n）" }
            },
            "required": ["action"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let risk = match args.action.as_str() {
            "list_tabs" => ToolRisk::SafeRead,
            "send_terminal_input" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        };
        invoke_app_tool(&self.ctx, Self::NAME, risk, args).await
    }
}

/// 领域 3 扩展：SFTP 目录同步与传输队列管理。
pub struct SftpTransferTool {
    ctx: ToolRuntimeContext,
}

impl SftpTransferTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for SftpTransferTool {
    const NAME: &'static str = "sftp_transfer";
    type Error = ToolExecutionError;
    type Args = AppActionArgs;
    type Output = String;

    fn description(&self) -> String {
        "管理 SFTP 传输队列与目录同步：list_transfers / cancel_transfer / sync_preview / sync_apply。"
            .to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_transfers", "cancel_transfer", "sync_preview", "sync_apply"]
                },
                "session_id": { "type": "string" },
                "tab_id": { "type": "string" },
                "transfer_id": { "type": "string" },
                "local_dir": { "type": "string" },
                "remote_dir": { "type": "string" },
                "direction": { "type": "string", "enum": ["upload", "download", "both"] }
            },
            "required": ["action"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let risk = if matches!(args.action.as_str(), "list_transfers" | "sync_preview") {
            ToolRisk::SafeRead
        } else {
            ToolRisk::Write
        };
        invoke_app_tool(&self.ctx, Self::NAME, risk, args).await
    }
}

/// 领域 4：主机监控与进程控制（本机/远端快照、进程列表、杀进程、采样开关、主机拨测）。
pub struct HostMonitorTool {
    ctx: ToolRuntimeContext,
}

impl HostMonitorTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for HostMonitorTool {
    const NAME: &'static str = "host_monitor";
    type Error = ToolExecutionError;
    type Args = AppActionArgs;
    type Output = String;

    fn description(&self) -> String {
        "主机监控与进程控制：snapshot / list_processes / kill_process / toggle_sampling / probe_host。"
            .to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["snapshot", "list_processes", "kill_process", "toggle_sampling", "probe_host"]
                },
                "session_id": { "type": "string", "description": "省略时针对本机，传入则针对指定会话" },
                "pid": { "type": "integer", "description": "kill_process 时的进程 PID" },
                "signal": { "type": "string", "enum": ["TERM", "KILL"], "description": "默认 TERM" },
                "enabled": { "type": "boolean", "description": "toggle_sampling 开关" },
                "host": { "type": "string", "description": "probe_host 目标主机" },
                "port": { "type": "integer", "description": "probe_host 目标端口" }
            },
            "required": ["action"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let risk = match args.action.as_str() {
            "snapshot" | "list_processes" | "probe_host" => ToolRisk::SafeRead,
            "kill_process" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        };
        invoke_app_tool(&self.ctx, Self::NAME, risk, args).await
    }
}

/// 领域 5：端口转发全生命周期管理（本地 / 远端 / 动态 SOCKS5）。
pub struct PortForwardOpsTool {
    ctx: ToolRuntimeContext,
}

impl PortForwardOpsTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for PortForwardOpsTool {
    const NAME: &'static str = "port_forward_ops";
    type Error = ToolExecutionError;
    type Args = AppActionArgs;
    type Output = String;

    fn description(&self) -> String {
        "管理 SSH 端口转发规则：list / create / start / stop / delete / probe。".to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "start", "stop", "delete", "probe"]
                },
                "session_id": { "type": "string" },
                "forward_id": { "type": "string" },
                "kind": { "type": "string", "enum": ["local", "remote", "dynamic"] },
                "listen_host": { "type": "string", "description": "默认 127.0.0.1" },
                "listen_port": { "type": "integer" },
                "target_host": { "type": "string" },
                "target_port": { "type": "integer" },
                "name": { "type": "string" }
            },
            "required": ["action"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let risk = match args.action.as_str() {
            "list" | "probe" => ToolRisk::SafeRead,
            "delete" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        };
        invoke_app_tool(&self.ctx, Self::NAME, risk, args).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rig_agent::tool::{Tool, ToolContext};
    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{AppActionArgs, SessionCatalogTool};
    use crate::output_store::ToolOutputStore;
    use crate::session::SessionScope;
    use crate::tools::{AppActionBridge, AppBridgeDomain, ToolError, ToolRuntimeContext};

    struct FakeBridge;

    #[async_trait]
    impl AppActionBridge for FakeBridge {
        async fn invoke(
            &self,
            domain: AppBridgeDomain,
            action: &str,
            args: Value,
        ) -> Result<String, ToolError> {
            Ok(format!(
                "ok:{domain}:{action}:{}",
                args["name"].as_str().unwrap_or("")
            ))
        }
    }

    #[tokio::test]
    async fn invokes_app_bridge_and_emits_card() {
        let (card_tx, mut card_rx) = mpsc::channel(8);
        let ctx = ToolRuntimeContext {
            scope: SessionScope::Global,
            locked_session: None,
            bulk_hosts: Vec::new(),
            output_store: Arc::new(ToolOutputStore::new()),
            command_timeout: Duration::from_secs(30),
            cancel: CancellationToken::new(),
            card_tx,
            plan_tx: None,
            remote_exec: None,
            remote_fs: None,
            terminal: None,
            runtime_inspector: None,
            forwarder: None,
            memory_writer: None,
            knowledge_searcher: None,
            app_bridge: Some(Arc::new(FakeBridge)),
            subagent_runner: None,
            skill_loader: None,
        };

        let tool = SessionCatalogTool::new(ctx);
        let mut tool_ctx = ToolContext::new();
        let mut extra = serde_json::Map::new();
        extra.insert("name".to_owned(), json!("prod-api"));
        let out = tool
            .call(
                &mut tool_ctx,
                AppActionArgs {
                    action: "create".to_owned(),
                    extra,
                },
            )
            .await
            .expect("调用必须成功");
        assert!(out.contains("ok:session_catalog:create:prod-api"));

        let card = card_rx.try_recv().expect("必须产出执行卡片");
        assert_eq!(card.tool_name, "session_catalog");
        assert!(card.ok);
        let _ = HashMap::<String, String>::new();
    }
}
