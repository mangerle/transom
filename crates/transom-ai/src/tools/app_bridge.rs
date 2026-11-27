//! 软件全局操作抽象桥接 Trait。
//!
//! 设计原理（Why）：
//! - `transom-ai` 仅定义纯数据契约，不依赖 GPUI、窗口实体或控制平面状态；
//! - 通过 [`AppActionBridge`] 将软件 10 大领域操作统一抽象为 `(domain, action, args)`，
//!   使得内置 AI 与外部 ACP（MCP Server）共用同一套实现、审批策略与审计日志。

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;

/// 软件级操作执行器抽象（由控制平面与 UI 联合实现）。
#[async_trait]
pub trait AppActionBridge: Send + Sync {
    /// 执行指定领域与动作的软件操作，返回面向模型的可读文本或 JSON 摘要。
    async fn invoke(&self, domain: &str, action: &str, args: Value) -> Result<String, ToolError>;
}
