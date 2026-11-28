//! AI 工具契约、后端抽象 Trait、动态风险分级与按场景装配。
//!
//! 设计原理（Why）：
//! - AI 助手面向全局工作区而非单一标签页：`remote_exec`、`remote_fs`、`transfer_file` 与
//!   `container_workspace` 始终可用，统一通过可选 `target` 参数路由到本机（`"local"`）、
//!   任意已保存远端服务器（未连接时自动触发连接）或默认当前活动 SSH 标签页；
//! - 通过抽象 Trait 解耦底层 `SshRemote` / `SftpClient` / `ContainerService` / GPUI 终端，
//!   使得本 crate 可在无网络、无 GUI 的单元测试中完整运行与验证。

pub mod app_bridge;
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

pub use registry::attach_tools;
pub(crate) use registry::export_mcp_tool_schemas;
pub use target::{ResolvedToolTarget, ToolTargetResolver, target_argument_for_tool};

mod io;
mod runtime_context;
pub use io::{ToolIoContext, with_io_guard};
pub use runtime_context::{ToolCapabilities, ToolProgressParams, ToolRuntimeContext};

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio_util::sync::CancellationToken;

use crate::ToolCallId;
use crate::compaction::{DEFAULT_TOOL_OUTPUT_LIMIT, SessionPromptContext, truncate_tool_output};
use crate::error::ToolError;
use crate::memory::MemoryProposal;
use crate::output_store::ToolOutputStore;
use crate::policy::{HostProtection, ToolRisk};
use crate::runner::AgentEvent;
use crate::session::SessionScope;
use crate::tools::plan::PlanSnapshot;

pub use app_bridge::{AppActionBridge, AppBridgeDomain};
pub use assess::{assess_tool_call, assess_tool_call_value};
pub use delegate::{DelegateMode, DelegateTaskRequest, SubagentModelTier, SubagentRunner};
pub use skill_tool::SkillLoader;

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

/// 远端命令执行结果。
#[derive(Clone, Debug)]
pub struct ExecCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 远端命令实时输出进度回调。
pub type ExecProgressCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// 远端命令执行器抽象（由控制平面复用 `SshRemote` 实现）。
#[async_trait]
pub trait RemoteExecutor: Send + Sync {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<ExecCommandResult, ToolError>;

    /// 带实时输出流预览的远端命令执行（默认回退到 `exec`）。
    async fn exec_with_progress(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
        _on_progress: ExecProgressCallback,
    ) -> Result<ExecCommandResult, ToolError> {
        self.exec(command, timeout, cancel).await
    }
}

/// 远端文件系统与传输执行器抽象（由控制平面复用 `SftpClient` + `sudo_write` 实现）。
#[async_trait]
pub trait RemoteFsExecutor: Send + Sync {
    async fn list_dir(&self, path: &str) -> Result<String, ToolError>;
    /// 探测远端路径是否存在。
    ///
    /// 与直接读取文件不同，调用方必须能区分“确认不存在”和“探测失败”，
    /// 否则检查点可能把权限错误误记为待删除的新文件。
    async fn file_exists(&self, path: &str) -> Result<bool, ToolError>;
    async fn read_file(&self, path: &str, max_bytes: usize) -> Result<String, ToolError>;
    async fn write_file(&self, path: &str, content: &str, backup: bool) -> Result<(), ToolError>;
    async fn mkdir(&self, path: &str) -> Result<(), ToolError>;
    async fn search_text(&self, path: &str, pattern: &str) -> Result<String, ToolError>;
    async fn transfer(
        &self,
        direction: TransferDirection,
        local_path: &str,
        remote_path: &str,
        overwrite: bool,
        cancel: CancellationToken,
    ) -> Result<String, ToolError>;
    async fn rename(&self, _from: &str, _to: &str) -> Result<(), ToolError> {
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前后端未实现远端重命名/移动".to_owned(),
        })
    }
    async fn delete(&self, _path: &str, _recursive: bool) -> Result<(), ToolError> {
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前后端未实现远端删除".to_owned(),
        })
    }
    async fn chmod(&self, _path: &str, _mode: u32) -> Result<(), ToolError> {
        Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: "当前后端未实现远端权限修改".to_owned(),
        })
    }

    /// 带统一截止时间与取消门的目录读取。
    async fn list_dir_with_io(&self, path: &str, io: &ToolIoContext) -> Result<String, ToolError> {
        with_io_guard("remote_fs", io, self.list_dir(path)).await
    }

    /// 带统一截止时间与取消门的路径探测。
    async fn file_exists_with_io(&self, path: &str, io: &ToolIoContext) -> Result<bool, ToolError> {
        with_io_guard("remote_fs", io, self.file_exists(path)).await
    }

    /// 带统一截止时间与取消门的文件读取。
    async fn read_file_with_io(
        &self,
        path: &str,
        max_bytes: usize,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard("remote_fs", io, self.read_file(path, max_bytes)).await
    }

    /// 带统一截止时间与取消门的文件写入。
    async fn write_file_with_io(
        &self,
        path: &str,
        content: &str,
        backup: bool,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.write_file(path, content, backup)).await
    }

    /// 带统一截止时间与取消门的目录创建。
    async fn mkdir_with_io(&self, path: &str, io: &ToolIoContext) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.mkdir(path)).await
    }

    /// 带统一截止时间与取消门的文本检索。
    async fn search_text_with_io(
        &self,
        path: &str,
        pattern: &str,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard("remote_fs", io, self.search_text(path, pattern)).await
    }

    /// 带统一截止时间与取消门的文件传输。
    async fn transfer_with_io(
        &self,
        direction: TransferDirection,
        local_path: &str,
        remote_path: &str,
        overwrite: bool,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard(
            "transfer_file",
            io,
            self.transfer(
                direction,
                local_path,
                remote_path,
                overwrite,
                io.cancel.clone(),
            ),
        )
        .await
    }

    /// 带统一截止时间与取消门的重命名。
    async fn rename_with_io(
        &self,
        from: &str,
        to: &str,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.rename(from, to)).await
    }

    /// 带统一截止时间与取消门的删除。
    async fn delete_with_io(
        &self,
        path: &str,
        recursive: bool,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.delete(path, recursive)).await
    }

    /// 带统一截止时间与取消门的权限修改。
    async fn chmod_with_io(
        &self,
        path: &str,
        mode: u32,
        io: &ToolIoContext,
    ) -> Result<(), ToolError> {
        with_io_guard("remote_fs", io, self.chmod(path, mode)).await
    }
}

/// 传输方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Upload,
    Download,
}

/// 终端上下文提供者（读取 OSC 133 命令块或当前屏幕末尾行）。
#[async_trait]
pub trait TerminalContextProvider: Send + Sync {
    async fn recent_blocks(&self, limit: usize) -> Result<String, ToolError>;
    async fn read_screen_tail(&self, lines: usize) -> Result<String, ToolError>;
}

/// 运行时监控与容器诊断执行器。
#[async_trait]
pub trait RuntimeInspector: Send + Sync {
    async fn host_runtime_snapshot(&self) -> Result<String, ToolError>;
    async fn container_action(
        &self,
        action: &str,
        container_id: Option<&str>,
        tail: Option<usize>,
    ) -> Result<String, ToolError>;

    /// 带统一截止时间与取消门的主机运行时快照。
    async fn host_runtime_snapshot_with_io(&self, io: &ToolIoContext) -> Result<String, ToolError> {
        with_io_guard("inspect_host_runtime", io, self.host_runtime_snapshot()).await
    }

    /// 带统一截止时间与取消门的容器动作。
    async fn container_action_with_io(
        &self,
        action: &str,
        container_id: Option<&str>,
        tail: Option<usize>,
        io: &ToolIoContext,
    ) -> Result<String, ToolError> {
        with_io_guard(
            "container_ops",
            io,
            self.container_action(action, container_id, tail),
        )
        .await
    }
}

/// 端口转发执行器。
#[async_trait]
pub trait ForwardExecutor: Send + Sync {
    async fn list_forwards(&self) -> Result<String, ToolError>;
    async fn create_forward(
        &self,
        listen_port: u16,
        target_host: &str,
        target_port: u16,
    ) -> Result<String, ToolError>;
    async fn stop_forward(&self, forward_id: &str) -> Result<String, ToolError>;
}

/// 记忆写入提议处理器。
#[async_trait]
pub trait MemoryWriter: Send + Sync {
    async fn propose_host_memory(&self, proposal: MemoryProposal) -> Result<String, ToolError>;
}

/// 本地知识库（长期记忆与自定义 Runbook）BM25 检索引擎抽象。
#[async_trait]
pub trait KnowledgeSearcher: Send + Sync {
    async fn search(&self, query: &str, top_k: usize) -> Result<String, ToolError>;
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
