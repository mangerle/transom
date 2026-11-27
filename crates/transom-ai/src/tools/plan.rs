//! 结构化规划、用户澄清提问与本地知识检索工具。

use std::time::Instant;

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::{ToolExecutionCard, ToolRuntimeContext, to_rig_err};
use crate::error::ToolError;
use crate::policy::ToolRisk;

/// 结构化任务步骤状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Done,
    Failed,
}

impl PlanStepStatus {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "in_progress" => Self::InProgress,
            "done" | "completed" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// 实时任务清单中的单条步骤。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub text: String,
    pub status: PlanStepStatus,
}

/// 推送给 UI 的实时任务清单快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSnapshot {
    pub title: Option<String>,
    pub steps: Vec<PlanStep>,
    pub focus: Option<String>,
}

#[derive(Deserialize)]
pub struct PlanStepInput {
    text: String,
    #[serde(default = "default_step_status")]
    status: String,
}

fn default_step_status() -> String {
    "pending".to_owned()
}

#[derive(Deserialize)]
pub struct UpdatePlanArgs {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    focus: Option<String>,
    steps: Vec<PlanStepInput>,
}

/// 在复杂多步运维任务中向用户与上下文同步结构化执行计划。
pub struct UpdatePlanTool {
    ctx: ToolRuntimeContext,
}

impl UpdatePlanTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for UpdatePlanTool {
    const NAME: &'static str = "update_plan";
    type Error = ToolExecutionError;
    type Args = UpdatePlanArgs;
    type Output = String;

    fn description(&self) -> String {
        "在多步部署、巡检或排障任务中更新结构化执行计划与当前步骤状态（pending / in_progress / done / failed）。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "可选：计划标题" },
                "focus": { "type": "string", "description": "可选：当前正在执行的简短状态描述" },
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "done", "failed"] }
                        },
                        "required": ["text"]
                    }
                }
            },
            "required": ["steps"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let started = Instant::now();
        let mut lines = Vec::with_capacity(args.steps.len() + 1);
        if let Some(title) = args.title.as_deref().filter(|t| !t.trim().is_empty()) {
            lines.push(format!("计划: {}", title.trim()));
        }
        let mut parsed_steps = Vec::with_capacity(args.steps.len());
        for (idx, step) in args.steps.iter().enumerate() {
            let st = PlanStepStatus::parse(&step.status);
            let tag = match st {
                PlanStepStatus::Done => "[已完成]",
                PlanStepStatus::InProgress => "[进行中]",
                PlanStepStatus::Failed => "[失败]",
                PlanStepStatus::Pending => "[待执行]",
            };
            lines.push(format!("{}. {tag} {}", idx + 1, step.text));
            parsed_steps.push(PlanStep {
                text: step.text.clone(),
                status: st,
            });
        }
        let rendered = lines.join("\n");
        if let Some(tx) = &self.ctx.plan_tx {
            let _ = tx.try_send(PlanSnapshot {
                title: args.title,
                steps: parsed_steps,
                focus: args.focus,
            });
        } else {
            self.ctx.record_card(ToolExecutionCard {
                tool_name: Self::NAME.to_owned(),
                host_label: String::new(),
                risk: ToolRisk::SafeRead,
                summary: args
                    .title
                    .unwrap_or_else(|| format!("更新执行计划（{} 步）", args.steps.len())),
                output: rendered.clone(),
                ok: true,
                approved: true,
                duration_ms: started.elapsed().as_millis() as u64,
                diff_preview: None,
            });
        }
        Ok(format!("执行计划已更新：\n{rendered}"))
    }
}

#[derive(Deserialize)]
pub struct RequestUserInputArgs {
    question: String,
    #[serde(default)]
    options: Vec<String>,
}

/// 当关键部署参数或危险决策存在歧义时，暂停并向用户发起结构化澄清提问。
pub struct RequestUserInputTool {
    ctx: ToolRuntimeContext,
}

impl RequestUserInputTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for RequestUserInputTool {
    const NAME: &'static str = "request_user_input";
    type Error = ToolExecutionError;
    type Args = RequestUserInputArgs;
    type Output = String;

    fn description(&self) -> String {
        "当关键参数不明确或存在多种高影响方案时调用本工具记录澄清问题，并在最终回复中向用户确认后等待下一轮输入。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "需要用户澄清的问题" },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "可选：提供给用户的推荐选项列表"
                }
            },
            "required": ["question"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let started = Instant::now();
        let detail = if args.options.is_empty() {
            args.question.clone()
        } else {
            format!("{}\n可选项: {}", args.question, args.options.join(" / "))
        };
        self.ctx.record_card(ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk: ToolRisk::SafeRead,
            summary: args.question.clone(),
            output: detail.clone(),
            ok: true,
            approved: true,
            duration_ms: started.elapsed().as_millis() as u64,
            diff_preview: None,
        });
        Ok(format!(
            "请在当前回复中直接向用户输出以下问题并停止继续猜测执行：\n{detail}"
        ))
    }
}

#[derive(Deserialize)]
pub struct SearchKnowledgeArgs {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

fn default_top_k() -> usize {
    5
}

/// 检索本地主机长期记忆与自定义运维 Runbook（基于纯 Rust BM25）。
pub struct SearchKnowledgeTool {
    ctx: ToolRuntimeContext,
}

impl SearchKnowledgeTool {
    pub fn new(ctx: ToolRuntimeContext) -> Self {
        Self { ctx }
    }
}

impl Tool for SearchKnowledgeTool {
    const NAME: &'static str = "search_knowledge";
    type Error = ToolExecutionError;
    type Args = SearchKnowledgeArgs;
    type Output = String;

    fn description(&self) -> String {
        "使用关键词检索本地保存的主机长期记忆与自定义运维 Runbook，获取历史经验或操作手册。"
            .to_owned()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "检索关键词（支持中英文）" },
                "top_k": { "type": "integer", "description": "最多返回条数，默认 5" }
            },
            "required": ["query"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let searcher = self
            .ctx
            .knowledge_searcher
            .as_ref()
            .ok_or_else(|| to_rig_err(ToolError::NoActiveSession))?;
        let started = Instant::now();
        let out = searcher
            .search(&args.query, args.top_k.clamp(1, 10))
            .await
            .map_err(to_rig_err)?;
        let clipped = self.ctx.format_tool_output(Self::NAME, &out);
        self.ctx.record_card(ToolExecutionCard {
            tool_name: Self::NAME.to_owned(),
            host_label: String::new(),
            risk: ToolRisk::SafeRead,
            summary: format!("检索知识库: {}", args.query),
            output: clipped.clone(),
            ok: true,
            approved: true,
            duration_ms: started.elapsed().as_millis() as u64,
            diff_preview: None,
        });
        Ok(clipped)
    }
}
