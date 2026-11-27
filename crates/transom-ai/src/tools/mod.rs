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
pub mod delegate;
pub mod extra;
pub mod ops;
pub mod plan;
pub mod remote;
pub mod runtime_ops;
pub mod skill_tool;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::compaction::{DEFAULT_TOOL_OUTPUT_LIMIT, SessionPromptContext};
use crate::error::ToolError;
use crate::memory::MemoryProposal;
use crate::output_store::ToolOutputStore;
use crate::policy::{HostProtection, ToolRisk};
use crate::session::SessionScope;

pub use app_bridge::AppActionBridge;
pub use assess::{assess_tool_call, assess_tool_call_value};
pub use delegate::{DelegateMode, DelegateTaskRequest, SubagentRunner};
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

/// 工具执行完成后推送给控制平面与 UI 的卡片载荷。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExecutionCard {
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
}

/// 远端命令执行结果。
#[derive(Clone, Debug)]
pub struct ExecCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 远端命令执行器抽象（由控制平面复用 `SshRemote` 实现）。
#[async_trait]
pub trait RemoteExecutor: Send + Sync {
    async fn exec(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<ExecCommandResult, ToolError>;
}

/// 远端文件系统与传输执行器抽象（由控制平面复用 `SftpClient` + `sudo_write` 实现）。
#[async_trait]
pub trait RemoteFsExecutor: Send + Sync {
    async fn list_dir(&self, path: &str) -> Result<String, ToolError>;
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

/// 工具运行上下文：汇集绑定会话、超时配置、取消令牌与各后端句柄。
#[derive(Clone)]
pub struct ToolRuntimeContext {
    pub scope: SessionScope,
    pub locked_session: Option<LockedSession>,
    pub bulk_hosts: Vec<BulkHostTarget>,
    pub output_store: Arc<ToolOutputStore>,
    pub command_timeout: Duration,
    pub cancel: CancellationToken,
    pub card_tx: mpsc::Sender<ToolExecutionCard>,
    pub plan_tx: Option<mpsc::Sender<plan::PlanSnapshot>>,
    pub remote_exec: Option<Arc<dyn RemoteExecutor>>,
    pub remote_fs: Option<Arc<dyn RemoteFsExecutor>>,
    pub terminal: Option<Arc<dyn TerminalContextProvider>>,
    pub runtime_inspector: Option<Arc<dyn RuntimeInspector>>,
    pub forwarder: Option<Arc<dyn ForwardExecutor>>,
    pub memory_writer: Option<Arc<dyn MemoryWriter>>,
    pub knowledge_searcher: Option<Arc<dyn KnowledgeSearcher>>,
    pub app_bridge: Option<Arc<dyn AppActionBridge>>,
    pub subagent_runner: Option<Arc<dyn SubagentRunner>>,
    pub skill_loader: Option<Arc<dyn SkillLoader>>,
}

impl ToolRuntimeContext {
    pub fn host_label(&self) -> String {
        self.locked_session
            .as_ref()
            .map(|s| s.prompt_ctx.host_name.clone())
            .unwrap_or_else(|| "通用会话".to_owned())
    }

    pub fn protection(&self) -> HostProtection {
        self.locked_session
            .as_ref()
            .map(|s| s.prompt_ctx.protection)
            .unwrap_or(HostProtection::Normal)
    }

    pub fn record_card(&self, mut card: ToolExecutionCard) {
        if card.host_label.is_empty() {
            card.host_label = self.host_label();
        }
        let _ = self.card_tx.try_send(card);
    }

    /// 清洗 ANSI 并通过会话级输出句柄池生成预览（超限则保留 `output_id`）。
    pub fn format_tool_output(&self, tool_name: &str, raw: &str) -> String {
        self.output_store
            .store_and_preview(tool_name, raw, DEFAULT_TOOL_OUTPUT_LIMIT)
    }

    /// 构造仅用于导出工具 Schema 的空上下文（不关联任何连接或任务）。
    pub(crate) fn dummy_for_schema() -> Self {
        let (card_tx, _card_rx) = mpsc::channel(1);
        Self {
            scope: SessionScope::Global,
            locked_session: None,
            bulk_hosts: Vec::new(),
            output_store: Arc::new(ToolOutputStore::new()),
            command_timeout: Duration::from_secs(60),
            cancel: CancellationToken::new(),
            card_tx,
            plan_tx: None,
            remote_exec: None,
            remote_fs: None,
            terminal: None,
            runtime_inspector: None,
            forwarder: None,
            memory_writer: None,
            knowledge_searcher: None,
            app_bridge: None,
            subagent_runner: None,
            skill_loader: None,
        }
    }
}

/// 构造单个工具的 MCP JSON Schema 条目。
fn mcp_schema_entry<T: rig_agent::tool::Tool>(tool: &T) -> serde_json::Value {
    serde_json::json!({
        "name": T::NAME,
        "description": tool.description(),
        "inputSchema": tool.parameters(),
    })
}

/// 从真实 `Tool` 实现中统一导出反向 MCP Server 暴露的工具 Schema 清单，杜绝双份维护漂移。
pub(crate) fn export_mcp_tool_schemas() -> Vec<serde_json::Value> {
    let ctx = ToolRuntimeContext::dummy_for_schema();
    vec![
        mcp_schema_entry(&remote::RemoteExecTool::new(ctx.clone())),
        mcp_schema_entry(&extra::RemoteExecBulkTool::new(ctx.clone())),
        mcp_schema_entry(&ops::RemoteFsTool::new(ctx.clone())),
        mcp_schema_entry(&ops::TransferFileTool::new(ctx.clone())),
        mcp_schema_entry(&ops::InspectHostRuntimeTool::new(ctx.clone())),
        mcp_schema_entry(&ops::ContainerOpsTool::new(ctx.clone())),
        mcp_schema_entry(&remote::GetRecentBlocksTool::new(ctx.clone())),
        mcp_schema_entry(&remote::ReadTerminalTool::new(ctx.clone())),
        mcp_schema_entry(&extra::ReadToolOutputTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools::SessionCatalogTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools::WorkspaceControlTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools::SftpTransferTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools::HostMonitorTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools::PortForwardOpsTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools_dev::ContainerWorkspaceTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools_dev::GitWorkspaceTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools_dev::ApiClientTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools_dev::AppSettingsOpsTool::new(ctx.clone())),
        mcp_schema_entry(&app_tools_dev::AiMemorySkillsTool::new(ctx)),
    ]
}

/// 动态风险评估结果（供审批门 `Gate::on_dispatch` 使用）。
pub struct ToolRiskAssessment {
    pub risk: ToolRisk,
    pub force_confirm: bool,
    pub summary: ApprovalSummary,
}

/// 按当前作用域动态装配注册给 `rig::AgentBuilder` 的工具。
pub fn attach_tools(
    mut builder: rig_agent::AgentBuilder,
    ctx: &ToolRuntimeContext,
) -> rig_agent::AgentBuilder {
    builder = builder
        .tool(extra::ReadToolOutputTool::new(ctx.clone()))
        .tool(plan::UpdatePlanTool::new(ctx.clone()))
        .tool(plan::RequestUserInputTool::new(ctx.clone()));
    if ctx.knowledge_searcher.is_some() {
        builder = builder.tool(plan::SearchKnowledgeTool::new(ctx.clone()));
    }
    if ctx.subagent_runner.is_some() {
        builder = builder.tool(delegate::DelegateTaskTool::new(ctx.clone()));
    }
    if ctx.skill_loader.is_some() {
        builder = builder.tool(skill_tool::LoadSkillTool::new(ctx.clone()));
    }
    if ctx.app_bridge.is_some() {
        builder = builder
            .tool(app_tools::SessionCatalogTool::new(ctx.clone()))
            .tool(app_tools::WorkspaceControlTool::new(ctx.clone()))
            .tool(app_tools::SftpTransferTool::new(ctx.clone()))
            .tool(app_tools::HostMonitorTool::new(ctx.clone()))
            .tool(app_tools::PortForwardOpsTool::new(ctx.clone()))
            .tool(app_tools_dev::ContainerWorkspaceTool::new(ctx.clone()))
            .tool(app_tools_dev::GitWorkspaceTool::new(ctx.clone()))
            .tool(app_tools_dev::ApiClientTool::new(ctx.clone()))
            .tool(app_tools_dev::AppSettingsOpsTool::new(ctx.clone()))
            .tool(app_tools_dev::AiMemorySkillsTool::new(ctx.clone()));
    }
    if ctx.app_bridge.is_some() || ctx.remote_exec.is_some() {
        builder = builder.tool(remote::RemoteExecTool::new(ctx.clone()));
    }
    if ctx.app_bridge.is_some() || ctx.remote_fs.is_some() {
        builder = builder
            .tool(ops::RemoteFsTool::new(ctx.clone()))
            .tool(ops::TransferFileTool::new(ctx.clone()));
    }
    if matches!(ctx.scope, SessionScope::Host(_)) && ctx.locked_session.is_some() {
        builder = builder
            .tool(remote::GetRecentBlocksTool::new(ctx.clone()))
            .tool(remote::ReadTerminalTool::new(ctx.clone()))
            .tool(remote::ProposeHostMemoryTool::new(ctx.clone()))
            .tool(ops::InspectHostRuntimeTool::new(ctx.clone()))
            .tool(ops::ContainerOpsTool::new(ctx.clone()))
            .tool(extra::PortForwardTool::new(ctx.clone()));
    }
    if ctx.bulk_hosts.len() >= 2 {
        builder = builder.tool(extra::RemoteExecBulkTool::new(ctx.clone()));
    }
    builder
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
