//! 按需加载运维技能正文与附带资源的工具（`load_skill`，实现 Agent Skills 渐进式披露）。

use std::time::Instant;

use async_trait::async_trait;
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::{ToolRuntimeContext, to_rig_err};
use transom_ai_contracts::error::ToolError;
use transom_ai_contracts::policy::ToolRisk;

/// 技能按需加载后端抽象（由控制平面实现）。
#[async_trait]
pub trait SkillLoader: Send + Sync {
    async fn load_skill(&self, name: &str, resource: Option<&str>) -> Result<String, ToolError>;
}

#[derive(Deserialize)]
pub struct LoadSkillArgs {
    name: String,
    #[serde(default)]
    resource: Option<String>,
}

/// 在命中系统提示词中的技能摘要时，按需加载完整 `SKILL.md` 正文或同目录参考资源。
pub struct LoadSkillTool {
    ctx: ToolRuntimeContext,
}

impl LoadSkillTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl super::ToolMetadata for LoadSkillTool {
    const NAME: &'static str = "load_skill";

    fn static_description() -> &'static str {
        "按需加载系统提示词中列出的某个运维技能（Skill）的完整 SOP 指令，或读取其附带的 references/ 与 scripts/ 参考文件。"
    }

    fn static_parameters() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "技能标识（id 或 /command）"
                },
                "resource": {
                    "type": "string",
                    "description": "可选：要读取的附带资源相对路径（如 references/checklist.md）"
                }
            },
            "required": ["name"]
        })
    }
}

impl Tool for LoadSkillTool {
    const NAME: &'static str = "load_skill";
    type Error = ToolExecutionError;
    type Args = LoadSkillArgs;
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
        let loader = self.ctx.caps.skill_loader.as_ref().ok_or_else(|| {
            to_rig_err(ToolError::ExecutionFailed {
                tool: Self::NAME,
                reason: "当前运行上下文未配置技能加载器".to_owned(),
            })
        })?;
        let started = Instant::now();
        let summary = match &args.resource {
            Some(r) if !r.trim().is_empty() => format!("{} ({r})", args.name),
            _ => args.name.clone(),
        };
        let res = loader
            .load_skill(&args.name, args.resource.as_deref())
            .await;
        let elapsed = started.elapsed().as_millis() as u64;
        self.ctx
            .finish_tool_result(
                super::ToolCardMeta::new(Self::NAME, ToolRisk::SafeRead, &summary, elapsed),
                res,
            )
            .await
    }
}
