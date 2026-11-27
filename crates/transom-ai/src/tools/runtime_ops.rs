//! 主机运行时快照采集与容器诊断运维工具。

use std::time::Instant;

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::{ToolExecutionCard, ToolRuntimeContext, to_rig_err};
use crate::error::ToolError;
use crate::policy::ToolRisk;

#[derive(Deserialize)]
pub struct EmptyArgs {}

/// 采集当前主机系统指标与容器概览的只读工具。
pub struct InspectHostRuntimeTool {
    ctx: ToolRuntimeContext,
}

impl InspectHostRuntimeTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for InspectHostRuntimeTool {
    const NAME: &'static str = "inspect_host_runtime";
    type Error = ToolExecutionError;
    type Args = EmptyArgs;
    type Output = String;

    fn description(&self) -> String {
        "只读采集当前绑定主机的 CPU、内存、负载、磁盘占用及运行中容器概览。".to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let inspector = self
            .ctx
            .caps
            .runtime_inspector
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let started = Instant::now();
        let snapshot = inspector
            .host_runtime_snapshot()
            .await
            .map_err(to_rig_err)?;
        let clipped = self.ctx.format_tool_output(Self::NAME, &snapshot);
        self.ctx.record_card(ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk: ToolRisk::SafeRead,
            summary: "采集主机运行时快照".to_owned(),
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
pub struct ContainerOpsArgs {
    action: String,
    #[serde(default)]
    container_id: Option<String>,
    #[serde(default)]
    tail: Option<usize>,
}

/// 容器诊断与受控运维工具（list / inspect / logs / restart）。
pub struct ContainerOpsTool {
    ctx: ToolRuntimeContext,
}

impl ContainerOpsTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for ContainerOpsTool {
    const NAME: &'static str = "container_ops";
    type Error = ToolExecutionError;
    type Args = ContainerOpsArgs;
    type Output = String;

    fn description(&self) -> String {
        "查看或操作当前主机上的 Docker/Podman 容器：action 支持 list / inspect / logs / stats / restart。".to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "inspect", "logs", "stats", "restart"] },
                "container_id": { "type": "string" },
                "tail": { "type": "integer" }
            },
            "required": ["action"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let inspector = self
            .ctx
            .caps
            .runtime_inspector
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let risk = if matches!(args.action.as_str(), "list" | "inspect" | "logs" | "stats") {
            ToolRisk::SafeRead
        } else {
            ToolRisk::Write
        };
        let started = Instant::now();
        let out = inspector
            .container_action(&args.action, args.container_id.as_deref(), args.tail)
            .await
            .map_err(to_rig_err)?;
        let clipped = self.ctx.format_tool_output(Self::NAME, &out);
        self.ctx.record_card(ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk,
            summary: format!("容器 {}", args.action),
            output: clipped.clone(),
            ok: true,
            approved: true,
            duration_ms: started.elapsed().as_millis() as u64,
            diff_preview: None,
        });
        Ok(clipped)
    }
}
