//! 远端命令、OSC 133 命令块、终端读屏与主机记忆提议工具。

use std::time::Instant;

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::{AppBridgeDomain, ToolRuntimeContext, to_rig_err};
use crate::error::ToolError;
use crate::memory::MemoryProposal;
use crate::policy::{
    ToolRisk, classify_shell_command, detect_blocking_command, format_blocking_command_hint,
};

#[derive(Deserialize)]
pub struct RemoteExecArgs {
    command: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// 在本机或指定/当前远端主机上执行 Shell 命令。
pub struct RemoteExecTool {
    ctx: ToolRuntimeContext,
}

impl RemoteExecTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for RemoteExecTool {
    const NAME: &'static str = "remote_exec";
    type Error = ToolExecutionError;
    type Args = RemoteExecArgs;
    type Output = String;

    fn description(&self) -> String {
        "在本机（target=\"local\"）或远端主机上执行一条 Shell 命令。\
         支持通过 target 指定已保存服务器名/session_id/IP（未连接时自动触发连接并等待就绪）；\
         省略 target 则默认在当前活动 SSH 标签页执行。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "要执行的 Shell 命令文本"
                },
                "target": {
                    "type": "string",
                    "description": "可选目标：\"local\" 表示本机；服务器名称/session_id/IP 表示远端（未连接自动连接）；省略则使用当前活动 SSH 会话"
                },
                "cwd": {
                    "type": "string",
                    "description": "可选工作目录（主要作用于 target=\"local\"）"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "可选超时秒数（默认按系统配置，本机/远端长任务可调大）"
                }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        if let Some(hit) = detect_blocking_command(&args.command) {
            return Ok(format_blocking_command_hint(&hit));
        }
        let target_trimmed = args
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let use_bridge = target_trimmed.is_some() || self.ctx.remote_exec.is_none();

        let started = Instant::now();
        let risk = classify_shell_command(&args.command);
        let host_label = match target_trimmed {
            Some(t) if t.eq_ignore_ascii_case("local") => "本机 (Local)".to_owned(),
            Some(t) => t.to_owned(),
            None => String::new(),
        };

        if use_bridge {
            let bridge = self
                .ctx
                .app_bridge
                .as_ref()
                .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
            let payload = json!({
                "target": target_trimmed,
                "command": args.command,
                "cwd": args.cwd,
                "timeout_secs": args.timeout_secs,
            });
            let res = bridge.invoke(AppBridgeDomain::RemoteExec, "run", payload).await;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            return match res {
                Ok(raw) => {
                    let clipped = self.ctx.format_tool_output(Self::NAME, &raw);
                    self.ctx.record_card(crate::tools::ToolExecutionCard {
                        tool_name: Self::NAME.to_owned(),
                        host_label,
                        risk,
                        summary: args.command,
                        output: clipped.clone(),
                        ok: true,
                        approved: true,
                        duration_ms: elapsed_ms,
                        diff_preview: None,
                    });
                    Ok(clipped)
                }
                Err(err) => {
                    let msg = err.to_string();
                    self.ctx.record_card(crate::tools::ToolExecutionCard {
                        tool_name: Self::NAME.to_owned(),
                        host_label,
                        risk,
                        summary: args.command,
                        output: msg,
                        ok: false,
                        approved: true,
                        duration_ms: elapsed_ms,
                        diff_preview: None,
                    });
                    Err(to_rig_err(err))
                }
            };
        }

        let executor = self
            .ctx
            .remote_exec
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let timeout = args
            .timeout_secs
            .map(|s| std::time::Duration::from_secs(s.clamp(5, 600)))
            .unwrap_or(self.ctx.command_timeout);
        let res = executor
            .exec(&args.command, timeout, self.ctx.cancel.clone())
            .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        match res {
            Ok(out) => {
                let mut combined = String::new();
                if !out.stdout.is_empty() {
                    combined.push_str(&out.stdout);
                }
                if !out.stderr.is_empty() {
                    if !combined.is_empty() {
                        combined.push_str("\n[stderr]\n");
                    }
                    combined.push_str(&out.stderr);
                }
                let ok = out.exit_code == 0;
                let clipped = self.ctx.format_tool_output(Self::NAME, &combined);
                self.ctx.record_card(crate::tools::ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label: String::new(),
                    risk,
                    summary: args.command,
                    output: combined,
                    ok,
                    approved: true,
                    duration_ms: elapsed_ms,
                    diff_preview: None,
                });
                Ok(format!("退出码: {}\n{clipped}", out.exit_code))
            }
            Err(err) => {
                let msg = err.to_string();
                self.ctx.record_card(crate::tools::ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label: String::new(),
                    risk,
                    summary: args.command,
                    output: msg,
                    ok: false,
                    approved: true,
                    duration_ms: elapsed_ms,
                    diff_preview: None,
                });
                Err(to_rig_err(err))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct RecentBlocksArgs {
    #[serde(default = "default_block_limit")]
    limit: usize,
}

fn default_block_limit() -> usize {
    5
}

/// 读取当前终端最近的 OSC 133 语义命令块。
pub struct GetRecentBlocksTool {
    ctx: ToolRuntimeContext,
}

impl GetRecentBlocksTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for GetRecentBlocksTool {
    const NAME: &'static str = "get_recent_blocks";
    type Error = ToolExecutionError;
    type Args = RecentBlocksArgs;
    type Output = String;

    fn description(&self) -> String {
        "读取当前终端最近的 OSC 133 命令执行块（含命令、退出码、耗时与输出片段），用于快速定位失败命令。".to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "最多返回最近多少个命令块，默认 5"
                }
            }
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let provider = self
            .ctx
            .terminal
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let started = Instant::now();
        let raw = provider
            .recent_blocks(args.limit.clamp(1, 20))
            .await
            .map_err(to_rig_err)?;
        let clipped = self.ctx.format_tool_output(Self::NAME, &raw);
        self.ctx.record_card(crate::tools::ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk: ToolRisk::SafeRead,
            summary: format!("读取最近 {} 个命令块", args.limit),
            output: clipped.clone(),
            ok: true,
            approved: true,
            duration_ms: started.elapsed().as_millis() as u64,
            diff_preview: None,
        });
        Ok(clipped)
    }
}

#[derive(Deserialize)]
pub struct ReadTerminalArgs {
    #[serde(default = "default_screen_lines")]
    lines: usize,
}

fn default_screen_lines() -> usize {
    80
}

/// 按需读取当前终端屏幕末尾行。
pub struct ReadTerminalTool {
    ctx: ToolRuntimeContext,
}

impl ReadTerminalTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for ReadTerminalTool {
    const NAME: &'static str = "read_terminal";
    type Error = ToolExecutionError;
    type Args = ReadTerminalArgs;
    type Output = String;

    fn description(&self) -> String {
        "读取用户当前正在查看的终端屏幕末尾若干行。仅当用户明确要求查看终端输出时才允许调用。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "lines": {
                    "type": "integer",
                    "description": "读取末尾多少行，默认 80"
                }
            }
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let provider = self
            .ctx
            .terminal
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let started = Instant::now();
        let raw = provider
            .read_screen_tail(args.lines.clamp(10, 300))
            .await
            .map_err(to_rig_err)?;
        let clipped = self.ctx.format_tool_output(Self::NAME, &raw);
        self.ctx.record_card(crate::tools::ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk: ToolRisk::SensitiveRead,
            summary: format!("读取终端末尾 {} 行", args.lines),
            output: clipped.clone(),
            ok: true,
            approved: true,
            duration_ms: started.elapsed().as_millis() as u64,
            diff_preview: None,
        });
        Ok(clipped)
    }
}

#[derive(Deserialize)]
pub struct ProposeMemoryArgs {
    fact: String,
    #[serde(default)]
    category: String,
}

/// 提议往当前绑定主机的长期记忆中添加一条事实或约束。
pub struct ProposeHostMemoryTool {
    ctx: ToolRuntimeContext,
}

impl ProposeHostMemoryTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for ProposeHostMemoryTool {
    const NAME: &'static str = "propose_host_memory";
    type Error = ToolExecutionError;
    type Args = ProposeMemoryArgs;
    type Output = String;

    fn description(&self) -> String {
        "提议将当前主机的关键环境路径、服务名或运维约束写入该主机的长期记忆（需用户确认）。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "fact": {
                    "type": "string",
                    "description": "要记住的环境事实或约束，简洁明确"
                },
                "category": {
                    "type": "string",
                    "description": "分类标签，例如：路径、服务、约束"
                }
            },
            "required": ["fact"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let locked = self
            .ctx
            .locked_session
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let writer = self
            .ctx
            .memory_writer
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let started = Instant::now();
        let out = writer
            .propose_host_memory(MemoryProposal {
                host_id: locked.session_id.clone(),
                host_name: locked.prompt_ctx.host_name.clone(),
                fact: args.fact.clone(),
                category: args.category,
            })
            .await
            .map_err(to_rig_err)?;
        self.ctx.record_card(crate::tools::ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk: ToolRisk::Write,
            summary: format!("写入主机记忆: {}", args.fact),
            output: out.clone(),
            ok: true,
            approved: true,
            duration_ms: started.elapsed().as_millis() as u64,
            diff_preview: None,
        });
        Ok(out)
    }
}
