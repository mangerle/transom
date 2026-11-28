//! 扩展只读与多主机协同工具：`read_tool_output` 与 `remote_exec_bulk`。

use std::time::Instant;

use futures::StreamExt as _;
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::{ToolExecutionCard, ToolRuntimeContext, to_rig_err};
use transom_ai_contracts::error::ToolError;
use transom_ai_contracts::policy::{
    ToolRisk, classify_shell_command, detect_blocking_command, format_blocking_command_hint,
};

#[derive(Deserialize)]
pub struct ReadToolOutputArgs {
    output_id: String,
    #[serde(default)]
    offset_line: usize,
    #[serde(default)]
    limit_lines: usize,
    #[serde(default)]
    pattern: Option<String>,
}

/// 按行偏移或关键字分页读取超长工具输出句柄。
pub struct ReadToolOutputTool {
    ctx: ToolRuntimeContext,
}

impl ReadToolOutputTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl super::ToolMetadata for ReadToolOutputTool {
    const NAME: &'static str = "read_tool_output";

    fn static_description() -> &'static str {
        "当工具输出被截断并附带 `output_id` 时，使用本工具按行偏移或关键字检索读取完整内容。"
    }

    fn static_parameters() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "output_id": { "type": "string", "description": "截断提示中给出的 output_id" },
                "offset_line": { "type": "integer", "description": "起始行偏移（0-based），默认 0" },
                "limit_lines": { "type": "integer", "description": "返回行数（1..=300），默认 120" },
                "pattern": { "type": "string", "description": "可选关键字过滤（大小写不敏感）" }
            },
            "required": ["output_id"]
        })
    }
}

impl Tool for ReadToolOutputTool {
    const NAME: &'static str = "read_tool_output";
    type Error = ToolExecutionError;
    type Args = ReadToolOutputArgs;
    type Output = String;

    fn description(&self) -> String {
        <Self as super::ToolMetadata>::static_description().to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        <Self as super::ToolMetadata>::static_parameters()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let started = Instant::now();
        let out = self
            .ctx
            .output_store
            .read(
                &args.output_id,
                args.offset_line,
                args.limit_lines,
                args.pattern.as_deref(),
            )
            .map_err(to_rig_err)?;
        let summary = format!("读取输出句柄 {}", args.output_id);
        self.ctx
            .record_outcome_card(
                super::ToolCardMeta::new(
                    Self::NAME,
                    ToolRisk::SafeRead,
                    &summary,
                    started.elapsed().as_millis() as u64,
                ),
                out.clone(),
                true,
            )
            .await;
        Ok(out)
    }
}

#[derive(Deserialize)]
pub struct RemoteExecBulkArgs {
    command: String,
    #[serde(default)]
    host_names: Vec<String>,
}

/// 在用户显式授权的多主机 Scope 内并发执行同一条 Shell 命令。
pub struct RemoteExecBulkTool {
    ctx: ToolRuntimeContext,
}

impl RemoteExecBulkTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl super::ToolMetadata for RemoteExecBulkTool {
    const NAME: &'static str = "remote_exec_bulk";

    fn static_description() -> &'static str {
        "在当前会话已授权的多台主机上并发执行同一条 Shell 命令并汇总各主机结果。写操作每次均需人工确认。"
    }

    fn static_parameters() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "要在多台目标主机上执行的 Shell 命令" },
                "host_names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "可选：限定只对已授权 Scope 中的部分主机名执行；留空表示全部已授权主机"
                }
            },
            "required": ["command"]
        })
    }
}

impl Tool for RemoteExecBulkTool {
    const NAME: &'static str = "remote_exec_bulk";
    type Error = ToolExecutionError;
    type Args = RemoteExecBulkArgs;
    type Output = String;

    fn description(&self) -> String {
        <Self as super::ToolMetadata>::static_description().to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        <Self as super::ToolMetadata>::static_parameters()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        if let Some(hit) = detect_blocking_command(&args.command) {
            return Ok(format_blocking_command_hint(&hit));
        }

        let targets: Vec<_> = if args.host_names.is_empty() {
            self.ctx.bulk_hosts.to_vec()
        } else {
            let mut matched = Vec::with_capacity(args.host_names.len());
            for wanted in &args.host_names {
                let Some(found) = self
                    .ctx
                    .bulk_hosts
                    .iter()
                    .find(|h| h.host_name == *wanted || h.session_id == *wanted)
                else {
                    return Err(to_rig_err(ToolError::PolicyDenied {
                        reason: format!("主机 `{wanted}` 不在当前会话已授权的多主机协同 Scope 内"),
                    }));
                };
                matched.push(found.clone());
            }
            matched
        };

        if targets.is_empty() {
            return Err(to_rig_err(ToolError::NoActiveSession));
        }

        let started = Instant::now();
        let risk = classify_shell_command(&args.command);
        let timeout = self.ctx.command_timeout;
        let cancel = self.ctx.cancel.clone();
        let command = args.command.clone();
        let bulk_host_label = format!("批量 {} 台主机", targets.len());
        let call_id = self
            .ctx
            .emit_tool_started(Self::NAME, &bulk_host_label, risk, &args.command)
            .await;

        let results: Vec<(String, String, bool)> = futures::stream::iter(targets)
            .map(|target| {
                let cmd = command.clone();
                let tok = cancel.clone();
                async move {
                    let host_started = Instant::now();
                    let header = format!("{} ({})", target.host_name, target.login);
                    match target.executor.exec(&cmd, timeout, tok).await {
                        Ok(res) => {
                            let host_ms = host_started.elapsed().as_millis() as u64;
                            let mut body = res.stdout;
                            if !res.stderr.is_empty() {
                                if !body.is_empty() {
                                    body.push('\n');
                                }
                                body.push_str("[stderr] ");
                                body.push_str(&res.stderr);
                            }
                            let ok = res.exit_code == 0;
                            (
                                format!("{header} | exit={} | {host_ms}ms", res.exit_code),
                                body.trim().to_owned(),
                                ok,
                            )
                        }
                        Err(err) => {
                            let host_ms = host_started.elapsed().as_millis() as u64;
                            (
                                format!("{header} | exit=-1 | {host_ms}ms"),
                                format!("执行失败: {err}"),
                                false,
                            )
                        }
                    }
                }
            })
            .buffer_unordered(8)
            .collect()
            .await;

        let all_ok = results.iter().all(|(_, _, ok)| *ok);
        let combined = results
            .into_iter()
            .map(|(header, body, _)| format!("=== [{header}] ===\n{body}"))
            .collect::<Vec<_>>()
            .join("\n\n");

        let clipped = self.ctx.format_tool_output(Self::NAME, &combined);
        self.ctx
            .record_card(ToolExecutionCard {
                call_id: call_id.into(),
                running: false,
                tool_name: Self::NAME.to_owned(),
                host_label: bulk_host_label,
                risk,
                summary: args.command,
                output: clipped.clone(),
                ok: all_ok,
                approved: true,
                duration_ms: started.elapsed().as_millis() as u64,
                diff_preview: None,
                ..Default::default()
            })
            .await;
        Ok(clipped)
    }
}

#[derive(Deserialize)]
pub struct PortForwardArgs {
    action: String,
    #[serde(default)]
    listen_port: Option<u16>,
    #[serde(default)]
    target_host: Option<String>,
    #[serde(default)]
    target_port: Option<u16>,
    #[serde(default)]
    forward_id: Option<String>,
}

/// 端口转发管理工具（list / create / stop）。
pub struct PortForwardTool {
    ctx: ToolRuntimeContext,
}

impl PortForwardTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl super::ToolMetadata for PortForwardTool {
    const NAME: &'static str = "port_forward";

    fn static_description() -> &'static str {
        "查看、创建或停止当前主机的 SSH 端口转发：action 支持 list / create / stop。"
    }

    fn static_parameters() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "create", "stop"] },
                "listen_port": { "type": "integer" },
                "target_host": { "type": "string" },
                "target_port": { "type": "integer" },
                "forward_id": { "type": "string" }
            },
            "required": ["action"]
        })
    }
}

impl Tool for PortForwardTool {
    const NAME: &'static str = "port_forward";
    type Error = ToolExecutionError;
    type Args = PortForwardArgs;
    type Output = String;

    fn description(&self) -> String {
        <Self as super::ToolMetadata>::static_description().to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        <Self as super::ToolMetadata>::static_parameters()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let forwarder = self
            .ctx
            .caps
            .forwarder
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let started = Instant::now();
        let risk = if args.action == "list" {
            ToolRisk::SafeRead
        } else {
            ToolRisk::Write
        };
        let out = match args.action.as_str() {
            "list" => forwarder.list_forwards().await.map_err(to_rig_err)?,
            "create" => {
                let lp = args.listen_port.unwrap_or(0);
                let th = args.target_host.as_deref().unwrap_or("127.0.0.1");
                let tp = args.target_port.unwrap_or(0);
                forwarder
                    .create_forward(lp, th, tp)
                    .await
                    .map_err(to_rig_err)?
            }
            "stop" => {
                let id = args.forward_id.as_deref().unwrap_or("");
                forwarder.stop_forward(id).await.map_err(to_rig_err)?
            }
            other => {
                return Err(to_rig_err(ToolError::InvalidArguments {
                    tool: Self::NAME,
                    reason: format!("未知 action: {other}"),
                }));
            }
        };
        let summary = format!("端口转发 {}", args.action);
        self.ctx
            .record_outcome_card(
                super::ToolCardMeta::new(
                    Self::NAME,
                    risk,
                    &summary,
                    started.elapsed().as_millis() as u64,
                ),
                out.clone(),
                true,
            )
            .await;
        Ok(out)
    }
}
