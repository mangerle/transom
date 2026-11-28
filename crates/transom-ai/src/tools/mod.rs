//! AI 工具契约、后端抽象 Trait、动态风险分级与按场景装配。
//!
//! 设计原理（Why）：
//! - AI 助手面向全局工作区而非单一标签页：`remote_exec`、`remote_fs`、`transfer_file` 与
//!   `container_workspace` 始终可用，统一通过可选 `target` 参数路由到本机（`"local"`）、
//!   任意已保存远端服务器（未连接时自动触发连接）或默认当前活动 SSH 标签页；
//! - 通过抽象 Trait 解耦底层 `SshRemote` / `SftpClient` / `ContainerService` / GPUI 终端，
//!   使得本 crate 可在无网络、无 GUI 的单元测试中完整运行与验证。

pub mod app_bridge;
pub mod app_bridge_types;
pub mod app_tools;
pub mod app_tools_dev;
pub mod assess;
pub mod assess_app;
pub mod compare;
pub mod delegate;
pub mod extra;
pub mod ops;
pub mod plan;
mod registry;
pub mod remote;
pub mod runtime_ops;
pub mod skill_tool;
mod target;
mod traits;

pub(crate) use registry::export_mcp_tool_schemas;
pub use registry::{attach_tools, estimate_tool_schema_tokens};
pub use target::{ResolvedToolTarget, ToolTargetResolver, target_argument_for_tool};
pub use traits::{
    ExecCommandResult, ExecProgressCallback, ForwardExecutor, KnowledgeSearcher, MemoryWriter,
    RemoteExecutor, RemoteFsExecutor, RuntimeInspector, TerminalContextProvider, TransferDirection,
};

mod io;
mod runtime_context;
pub use io::{ToolIoContext, with_io_guard};
pub use runtime_context::{
    AppToolVisibility, ToolCapabilities, ToolCardMeta, ToolProgressParams, ToolRuntimeContext,
};

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ToolCallId;
use crate::compaction::{DEFAULT_TOOL_OUTPUT_LIMIT, SessionPromptContext, truncate_tool_output};
use transom_ai_contracts::error::ToolError;
use transom_ai_contracts::policy::{HostProtection, ToolRisk};

pub use app_bridge::{AppActionBridge, AppBridgeDomain};
pub use app_bridge_types::{
    AiMemorySkillsAction, ApiClientAction, AppSettingsOpsAction, ContainerWorkspaceAction,
    GitWorkspaceAction, HostMonitorAction, PortForwardOpsAction, SessionCatalogAction,
    SftpTransferAction, WorkspaceControlAction, parse_app_action,
};
pub use assess::{assess_tool_call, assess_tool_call_value};
pub use delegate::{DelegateMode, DelegateTaskRequest, SubagentModelTier, SubagentRunner};
pub use skill_tool::SkillLoader;

/// 工具静态元信息契约，使 MCP Schema 导出与 Token 估算无需构造 [`ToolRuntimeContext`] 实例。
pub trait ToolMetadata {
    const NAME: &'static str;
    fn static_description() -> &'static str;
    fn static_parameters() -> serde_json::Value;

    /// 导出 MCP JSON Schema 条目。
    fn mcp_schema() -> serde_json::Value {
        serde_json::json!({
            "name": Self::NAME,
            "description": Self::static_description(),
            "inputSchema": Self::static_parameters(),
        })
    }
}

/// 锁定在单次 Agent 运行期间的会话上下文。
#[derive(Clone, Debug)]
pub struct LockedSession {
    pub session_id: String,
    pub prompt_ctx: SessionPromptContext,
}

/// 审批请求摘要（展示在 UI 输入框上方的悬浮审批卡片中）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSummary {
    pub tool_name: String,
    pub host_label: String,
    pub protection: HostProtection,
    pub risk: ToolRisk,
    pub title: String,
    pub detail: String,
    #[serde(default)]
    pub diff_preview: Option<String>,
    /// 是否要求每次重新弹窗确认（例如多主机批量写操作）。
    #[serde(default)]
    pub requires_fresh_approval: bool,
}

/// 工具执行完成后（或运行中流式进度）推送给控制平面与 UI 的卡片载荷。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExecutionCard {
    #[serde(default)]
    pub call_id: ToolCallId,
    #[serde(default)]
    pub running: bool,
    pub tool_name: String,
    pub host_label: String,
    pub risk: ToolRisk,
    pub summary: String,
    pub output: String,
    pub ok: bool,
    pub approved: bool,
    pub duration_ms: u64,
    #[serde(default)]
    pub diff_preview: Option<String>,
    /// 远端文件写操作的目标路径（用于一键还原）。
    #[serde(default)]
    pub revert_target_path: Option<String>,
    /// 远端文件修改前在本机保存的快照路径（用于一键还原）。
    #[serde(default)]
    pub revert_backup_path: Option<String>,
}

impl ToolExecutionCard {
    /// 生成可进入 UI、会话记录、审计与 Hook 的安全快照。
    ///
    /// 所有工具输出都在这里统一完成敏感信息脱敏和长度限制，避免某个工具忘记调用
    /// `format_tool_output` 就把完整命令输出写入 JSONL。调用方仍可保留原始输出用于
    /// 本地 `output_id` 存储，但不能再把原始内容直接交给跨模块记录链路。
    pub fn sanitized(mut self) -> Self {
        self.summary = truncate_tool_output(&self.summary, 2_000);
        self.output = truncate_tool_output(&self.output, DEFAULT_TOOL_OUTPUT_LIMIT);
        self.diff_preview = self
            .diff_preview
            .as_deref()
            .map(|preview| truncate_tool_output(preview, 6_000));
        self
    }
}

impl Default for ToolExecutionCard {
    fn default() -> Self {
        Self {
            call_id: ToolCallId::default(),
            running: false,
            tool_name: String::new(),
            host_label: String::new(),
            risk: ToolRisk::SafeRead,
            summary: String::new(),
            output: String::new(),
            ok: true,
            approved: true,
            duration_ms: 0,
            diff_preview: None,
            revert_target_path: None,
            revert_backup_path: None,
        }
    }
}

/// 已授权加入多主机协同 Scope 的单个远端目标。
#[derive(Clone)]
pub struct BulkHostTarget {
    pub session_id: String,
    pub host_name: String,
    pub login: String,
    pub protection: HostProtection,
    pub executor: Arc<dyn RemoteExecutor>,
}

/// 动态风险评估结果（供审批门 `Gate::on_dispatch` 使用）。
pub struct ToolRiskAssessment {
    pub risk: ToolRisk,
    pub force_confirm: bool,
    pub summary: ApprovalSummary,
}

pub(crate) fn to_rig_err(err: ToolError) -> rig_agent::tool::ToolExecutionError {
    match err {
        ToolError::InvalidArguments { .. } => {
            rig_agent::tool::ToolExecutionError::invalid_args(err.to_string())
        }
        ToolError::ExecutionTimeout { .. } => {
            rig_agent::tool::ToolExecutionError::timeout(err.to_string())
        }
        ToolError::Cancelled => rig_agent::tool::ToolExecutionError::cancelled(err.to_string()),
        other => rig_agent::tool::ToolExecutionError::other(other.to_string()),
    }
}

/// 截取多行输出的末尾 `max_lines` 行，并将每行限长至 `max_chars` 字符，用于运行态流式预览。
pub(crate) fn clip_tail_lines(raw: &str, max_lines: usize, max_chars: usize) -> String {
    if raw.is_empty() || max_lines == 0 {
        return String::new();
    }
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..]
        .iter()
        .map(|line| {
            let mut chars = line.chars();
            let head: String = chars.by_ref().take(max_chars).collect();
            if chars.next().is_some() {
                format!("{head}…")
            } else {
                head
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::{ToolError, ToolExecutionCard, ToolIoContext, clip_tail_lines, with_io_guard};

    #[test]
    fn sanitizes_card_summary_output_and_diff() {
        let card = ToolExecutionCard {
            summary: "DB_PASSWORD=super-secret".to_owned(),
            output: format!(
                "Authorization: Bearer abcdefghijklmnopqrstuvwxyz\n{}",
                "x".repeat(20_000)
            ),
            diff_preview: Some("token=abcdefghijklmnopqrstuvwxyz".to_owned()),
            ..Default::default()
        }
        .sanitized();

        assert!(!card.summary.contains("super-secret"));
        assert!(!card.output.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(card.output.len() <= 9_000);
        assert!(
            card.diff_preview
                .as_deref()
                .is_some_and(|preview| !preview.contains("abcdefghijklmnopqrstuvwxyz"))
        );
    }

    #[test]
    fn clips_tail_lines_and_long_line() {
        let raw = "l1\n\nl2\nl3\nl4\nl5\nl6\n1234567890";
        let res = clip_tail_lines(raw, 3, 5);
        assert_eq!(res, "l5\nl6\n12345…");
    }

    #[tokio::test]
    async fn io_guard_converts_deadline_to_timeout_error() {
        let io = ToolIoContext::new(Duration::from_millis(5), CancellationToken::new());
        let result = with_io_guard("test_tool", &io, async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Err::<(), _>(ToolError::ExecutionFailed {
                tool: "test_tool",
                reason: "不应执行到此处".to_owned(),
            })
        })
        .await;
        assert!(matches!(result, Err(ToolError::ExecutionTimeout { .. })));
    }

    #[tokio::test]
    async fn io_guard_observes_cancellation() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let io = ToolIoContext::new(Duration::from_secs(1), cancel);
        let result = with_io_guard("test_tool", &io, async {
            Err::<(), _>(ToolError::ExecutionFailed {
                tool: "test_tool",
                reason: "不应执行到此处".to_owned(),
            })
        })
        .await;
        assert!(matches!(result, Err(ToolError::Cancelled)));
    }
}
