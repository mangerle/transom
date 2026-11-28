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

/// 统一委托 [`AppActionBridge`] 执行软件操作，前置校验合法 `action` 并记录工具卡片。
pub(crate) async fn invoke_app_tool(
    ctx: &ToolRuntimeContext,
    tool_name: &'static str,
    allowed_actions: &[&str],
    risk: ToolRisk,
    args: AppActionArgs,
) -> Result<String, ToolExecutionError> {
    if !allowed_actions.contains(&args.action.as_str()) {
        return Err(to_rig_err(ToolError::InvalidArguments {
            tool: tool_name,
            reason: format!(
                "不支持的 action `{}`，允许值为: {}",
                args.action,
                allowed_actions.join(" / ")
            ),
        }));
    }
    let bridge = ctx.caps.app_bridge.as_ref().ok_or_else(|| {
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
                ..Default::default()
            })
            .await;
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
                ..Default::default()
            })
            .await;
            Err(to_rig_err(err))
        }
    }
}

/// 声明式定义基于 [`AppActionBridge`] 的软件领域工具，统一实现 `ACTIONS` 前置校验与 `Tool` 特型。
macro_rules! define_app_tool {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            tool_name: $tool_name:literal,
            actions: [$($act:literal),+ $(,)?],
            description: $desc:expr,
            schema_props: $props:expr,
            risk: |$risk_act:ident| $risk_body:expr $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $name {
            ctx: ToolRuntimeContext,
        }

        impl $name {
            /// 工具在注册表、Schema 与风险分发中共用的稳定名称。
            pub const NAME: &'static str = $tool_name;

            /// 该工具允许的 `action` 枚举清单（单一事实来源，同时用于 Schema 导出与前置校验）。
            pub const ACTIONS: &'static [&'static str] = &[$($act),+];

            /// 按动作返回风险等级；执行卡片与审批门共享这一判定。
            pub(crate) fn risk_for(action: &str) -> ToolRisk {
                let $risk_act = action;
                $risk_body
            }

            pub fn new(ctx: ToolRuntimeContext) -> Self {
                Self { ctx }
            }
        }

        impl Tool for $name {
            const NAME: &'static str = $tool_name;
            type Error = ToolExecutionError;
            type Args = AppActionArgs;
            type Output = String;

            fn description(&self) -> String {
                $desc.to_owned()
            }

            fn parameters(&self) -> Value {
                let mut props = $props;
                if let Some(obj) = props.as_object_mut() {
                    obj.insert(
                        "action".to_owned(),
                        json!({
                            "type": "string",
                            "enum": Self::ACTIONS
                        }),
                    );
                }
                json!({
                    "type": "object",
                    "properties": props,
                    "required": ["action"]
                })
            }

            async fn call(
                &self,
                _context: &mut ToolContext,
                args: Self::Args,
            ) -> Result<Self::Output, Self::Error> {
                let risk = Self::risk_for(&args.action);
                invoke_app_tool(&self.ctx, Self::NAME, Self::ACTIONS, risk, args).await
            }
        }
    };
}

pub(crate) use define_app_tool;

define_app_tool! {
    /// 领域 1：会话与目录管理（增删改查、分组、known_hosts、部署公钥）。
    pub struct SessionCatalogTool {
        tool_name: "session_catalog",
        actions: [
            "list", "create", "update", "delete",
            "create_folder", "delete_folder",
            "list_known_hosts", "remove_known_host", "deploy_public_key"
        ],
        description: "管理 Transom 已保存会话目录与受信任主机：list / create / update / delete / \
         create_folder / delete_folder / list_known_hosts / remove_known_host / deploy_public_key。",
        schema_props: json!({
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
        }),
        risk: |act| {
            if matches!(act, "delete" | "delete_folder" | "remove_known_host" | "deploy_public_key") {
                ToolRisk::Destructive
            } else if matches!(act, "list" | "list_known_hosts") {
                ToolRisk::SafeRead
            } else {
                ToolRisk::Write
            }
        }
    }
}

define_app_tool! {
    /// 领域 2：终端与工作区控制（标签页、打开本地终端、连接会话、分屏、切换面板、发送输入）。
    pub struct WorkspaceControlTool {
        tool_name: "workspace_control",
        actions: [
            "list_tabs", "connect_session", "open_local_terminal",
            "switch_tab", "close_tab", "split_pane", "open_panel", "send_terminal_input"
        ],
        description: "控制 Transom 主窗口标签页与面板：list_tabs / connect_session / open_local_terminal / \
         switch_tab / close_tab / split_pane / open_panel / send_terminal_input。\
         注意：仅在用户明确要求打开窗口/终端或必须交互操作时才调用 connect_session；\
         常规远端巡检、命令与文件传输请直接通过执行工具的 target 参数在后台静默连接，不要打开标签页。",
        schema_props: json!({
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
        }),
        risk: |act| match act {
            "list_tabs" => ToolRisk::SafeRead,
            "send_terminal_input" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}

define_app_tool! {
    /// 领域 3 扩展：SFTP 目录同步与传输队列管理。
    pub struct SftpTransferTool {
        tool_name: "sftp_transfer",
        actions: ["list_transfers", "cancel_transfer", "sync_preview", "sync_apply"],
        description: "管理 SFTP 传输队列与目录同步：list_transfers / cancel_transfer / sync_preview / sync_apply。",
        schema_props: json!({
            "session_id": { "type": "string" },
            "tab_id": { "type": "string" },
            "transfer_id": { "type": "string" },
            "local_dir": { "type": "string" },
            "remote_dir": { "type": "string" },
            "direction": { "type": "string", "enum": ["upload", "download", "both"] }
        }),
        risk: |act| if matches!(act, "list_transfers" | "sync_preview") {
            ToolRisk::SafeRead
        } else {
            ToolRisk::Write
        }
    }
}

define_app_tool! {
    /// 领域 4：主机监控与进程控制（本机/远端快照、进程列表、杀进程、采样开关、主机拨测）。
    pub struct HostMonitorTool {
        tool_name: "host_monitor",
        actions: ["snapshot", "list_processes", "kill_process", "toggle_sampling", "probe_host"],
        description: "主机监控与进程控制：snapshot / list_processes / kill_process / toggle_sampling / probe_host。",
        schema_props: json!({
            "session_id": { "type": "string", "description": "省略时针对本机，传入则针对指定会话" },
            "pid": { "type": "integer", "description": "kill_process 时的进程 PID" },
            "signal": { "type": "string", "enum": ["TERM", "KILL"], "description": "默认 TERM" },
            "enabled": { "type": "boolean", "description": "toggle_sampling 开关" },
            "host": { "type": "string", "description": "probe_host 目标主机" },
            "port": { "type": "integer", "description": "probe_host 目标端口" }
        }),
        risk: |act| match act {
            "snapshot" | "list_processes" | "probe_host" => ToolRisk::SafeRead,
            "kill_process" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}

define_app_tool! {
    /// 领域 5：端口转发全生命周期管理（本地 / 远端 / 动态 SOCKS5）。
    pub struct PortForwardOpsTool {
        tool_name: "port_forward_ops",
        actions: ["list", "create", "start", "stop", "delete", "probe"],
        description: "管理 SSH 端口转发规则：list / create / start / stop / delete / probe。",
        schema_props: json!({
            "session_id": { "type": "string" },
            "forward_id": { "type": "string" },
            "kind": { "type": "string", "enum": ["local", "remote", "dynamic"] },
            "listen_host": { "type": "string", "description": "默认 127.0.0.1" },
            "listen_port": { "type": "integer" },
            "target_host": { "type": "string" },
            "target_port": { "type": "integer" },
            "name": { "type": "string" }
        }),
        risk: |act| match act {
            "list" | "probe" => ToolRisk::SafeRead,
            "delete" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
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
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let mut ctx =
            ToolRuntimeContext::minimal(SessionScope::Global, event_tx, CancellationToken::new());
        ctx.command_timeout = Duration::from_secs(30);
        ctx.caps.app_bridge = Some(Arc::new(FakeBridge));

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

        let event = event_rx.try_recv().expect("必须产出执行卡片");
        let crate::runner::AgentEvent::ToolCard(card) = event else {
            panic!("期望工具卡片事件");
        };
        assert_eq!(card.tool_name, "session_catalog");
        assert!(card.ok);
        let _ = HashMap::<String, String>::new();
    }
}
