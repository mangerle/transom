//! AI 运维审计留痕（只追加 JSONL）。
//!
//! 设计原理（Why）：
//! - 记录谁在什么时间、针对哪台主机、经 AI 提议并批准/拒绝了什么动作及退出结果；
//! - 绝不记录明文 API Key、密码或完整敏感输出，与 Transom 的安全与可观测性规范对齐。

use serde::{Deserialize, Serialize};

use crate::error::AiError;
use crate::policy::ToolRisk;

/// 单条 AI 运维审计记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiAuditEntry {
    pub timestamp: String,
    pub conversation_id: String,
    pub host_label: String,
    pub tool_name: String,
    pub risk: ToolRisk,
    pub summary: String,
    pub approved: bool,
    pub ok: bool,
    pub duration_ms: u64,
}

impl AiAuditEntry {
    /// 序列化为单行 JSONL 文本；磁盘权限与落盘由存储层负责。
    pub fn to_json_line(&self) -> Result<String, AiError> {
        serde_json::to_string(self).map_err(|err| AiError::StorageFailed {
            target: "序列化 AI 审计记录".to_owned(),
            reason: err.to_string(),
        })
    }
}
