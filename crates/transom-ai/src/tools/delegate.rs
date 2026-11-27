//! 按需子 Agent 委派工具（`delegate_task`）。
//!
//! 设计原理（Why）：
//! - 将多主机并行只读调研或独立子部署任务交给独立上下文的子 Agent 执行，
//!   防止大量日志/探测输出污染主对话窗口；
//! - 子 Agent 强制剥夺 `subagent_runner`（禁止递归派生），且在控制平面受独立
//!   托管任务 `CancellationToken` 与 `ReadOnly`/`Deploy` 策略门控。

use std::time::Instant;

use async_trait::async_trait;
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ToolExecutionCard, ToolRuntimeContext, to_rig_err};
use crate::error::ToolError;
use crate::policy::ToolRisk;

/// 子 Agent 执行模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateMode {
    #[default]
    Readonly,
    Deploy,
}

/// 派发子 Agent 的任务请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegateTaskRequest {
    pub title: String,
    pub task: String,
    pub mode: DelegateMode,
    pub target: Option<String>,
    pub max_turns: usize,
}

/// 子 Agent 运行器抽象（由控制平面实现并注册至托管任务中心）。
#[async_trait]
pub trait SubagentRunner: Send + Sync {
    async fn run_subagent(&self, req: DelegateTaskRequest) -> Result<String, ToolError>;
}

#[derive(Deserialize)]
pub struct DelegateTaskArgs {
    title: String,
    task: String,
    #[serde(default)]
    mode: DelegateMode,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    max_turns: Option<usize>,
}

/// 在独立上下文中派发只读调研或独立部署子任务，并汇总结果回主对话。
pub struct DelegateTaskTool {
    ctx: ToolRuntimeContext,
}

impl DelegateTaskTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for DelegateTaskTool {
    const NAME: &'static str = "delegate_task";
    type Error = ToolExecutionError;
    type Args = DelegateTaskArgs;
    type Output = String;

    fn description(&self) -> String {
        "在独立上下文中启动一个子 Agent 执行复杂只读调研（mode=\"readonly\"）或独立部署子任务（mode=\"deploy\"），\
         避免大量中间输出撑爆主上下文；可并行多次调用以同时排查多台服务器。".to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "简短清晰的子任务标题（将显示在运行中任务列表）"
                },
                "task": {
                    "type": "string",
                    "description": "完整的子任务目标、约束与期望返回的汇总格式"
                },
                "mode": {
                    "type": "string",
                    "enum": ["readonly", "deploy"],
                    "description": "readonly 仅只读调研（自动拦截一切写操作）；deploy 允许写与部署（仍受审批策略保护）"
                },
                "target": {
                    "type": "string",
                    "description": "可选默认目标（\"local\" 或服务器名/ID），仅用于上下文提示与卡片展示"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "子 Agent 最大轮次，默认 8（上限 15）"
                }
            },
            "required": ["title", "task"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let runner = self.ctx.caps.subagent_runner.as_ref().ok_or_else(|| {
            to_rig_err(ToolError::ExecutionFailed {
                tool: Self::NAME,
                reason: "当前运行上下文不支持子 Agent 派发".to_owned(),
            })
        })?;
        let risk = match args.mode {
            DelegateMode::Readonly => ToolRisk::SafeRead,
            DelegateMode::Deploy => ToolRisk::Write,
        };
        let host_label = args
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("子 Agent")
            .to_owned();
        let req = DelegateTaskRequest {
            title: args.title.clone(),
            task: args.task,
            mode: args.mode,
            target: args.target,
            max_turns: args.max_turns.unwrap_or(8).clamp(2, 15),
        };
        let started = Instant::now();
        let res = runner.run_subagent(req).await;
        let elapsed = started.elapsed().as_millis() as u64;
        match res {
            Ok(out) => {
                let clipped = self.ctx.format_tool_output(Self::NAME, &out);
                self.ctx.record_card(ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label,
                    risk,
                    summary: args.title,
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
                self.ctx.record_card(ToolExecutionCard {
                    tool_name: Self::NAME.to_owned(),
                    host_label,
                    risk,
                    summary: args.title,
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
}
