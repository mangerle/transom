//! AI 助手控制平面命令与事件总线 DTO。
//!
//! 设计原理（Why）：
//! - 保持 `transom-core` 轻量且不依赖 `transom-ai` / `rig-core`，仅传递界面与控制平面
//!   交互所需的结构化摘要、流式增量与审批响应。

use serde::{Deserialize, Serialize};

use crate::ids::{SessionId, TabId};
use crate::model::session::HostProtection;

/// 托管后台任务的运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiManagedTaskStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// 托管后台任务摘要（由控制平面推送给 AI 面板任务中心）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiManagedTaskDto {
    pub task_id: String,
    pub kind: String,
    pub title: String,
    pub target_label: String,
    pub status: AiManagedTaskStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

/// UI 请求终端上下文的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiTerminalContextKind {
    RecentBlocks { limit: usize },
    ScreenTail { lines: usize },
}

/// 单个任务计划步骤的执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPlanStepStatus {
    Pending,
    InProgress,
    Done,
    Failed,
}

/// 实时任务清单中的单条步骤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPlanStepDto {
    pub text: String,
    pub status: AiPlanStepStatus,
}

/// 实时任务清单卡片快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPlanDto {
    pub title: Option<String>,
    pub steps: Vec<AiPlanStepDto>,
    pub focus: Option<String>,
}

/// 用户对审批卡片做出的结构化裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiApprovalDecision {
    /// 仅本次允许执行。
    AllowOnce,
    /// 在当前会话中自动放行同前缀的安全/常规命令。
    AllowPrefix { prefix: String },
    /// 拒绝执行并附带修改意见，驱动 AI 立即调整方案。
    RejectWithFeedback { reason: String },
    /// 移交到交互式终端由用户手动执行，挂起等待“继续”。
    HandoffToTerminal,
}

/// 上下文窗口的 Token 构成拆解。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiContextBreakdownDto {
    pub system_tokens: u32,
    pub memory_tokens: u32,
    pub tools_tokens: u32,
    pub history_tokens: u32,
    pub tool_output_tokens: u32,
    pub context_window: u32,
}

/// 待确认的审批卡片摘要（由控制平面发给 UI）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiApprovalDto {
    pub request_id: String,
    pub run_id: String,
    pub tool_name: String,
    pub host_label: String,
    pub protection: HostProtection,
    pub risk_label: String,
    pub is_destructive: bool,
    pub title: String,
    pub detail: String,
    pub diff_preview: Option<String>,
    pub requires_fresh_approval: bool,
    /// 建议允许的命令前缀（如 `docker ps`、`systemctl status`），为空则不显示“本会话允许同类”。
    pub suggested_prefix: Option<String>,
    /// 执行前影响说明（高危或写操作的影响面/不可逆性提示）。
    pub impact_note: Option<String>,
}

/// 已授权参与多主机协同的远端目标描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiHostTargetDto {
    pub tab_id: TabId,
    pub session_id: SessionId,
    pub host_name: String,
    pub login: String,
    pub platform: String,
    pub protection: HostProtection,
}

/// 已执行的工具结果卡片 DTO。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiToolCardDto {
    pub conversation_id: String,
    pub tool_name: String,
    pub host_label: String,
    pub risk_label: String,
    pub summary: String,
    pub output: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub diff_preview: Option<String>,
}

/// 发起提问的请求载荷（装箱以保持 `AiCommand` 体积紧凑）。
#[derive(Debug, Clone)]
pub struct AiPromptRequest {
    pub run_id: String,
    pub conversation_id: Option<String>,
    pub tab_id: Option<TabId>,
    pub session_id: Option<SessionId>,
    pub host_name: Option<String>,
    pub login: Option<String>,
    pub platform: Option<String>,
    pub protection: HostProtection,
    pub model_id: String,
    pub effort: Option<String>,
    pub prompt: String,
    pub images_base64: Vec<(String, String)>,
    pub attachment_text: Option<(String, String)>,
    pub extra_hosts: Vec<AiHostTargetDto>,
    pub acp_agent_id: Option<String>,
    /// 是否处于仅只读规划（Plan）模式。
    pub plan_mode: bool,
}

/// UI 发送给控制平面的 AI 命令。
#[derive(Debug, Clone)]
pub enum AiCommand {
    /// 发起一次新提问。
    SendPrompt(Box<AiPromptRequest>),
    /// 停止指定运行。
    CancelRun { run_id: String },
    /// 用户对审批卡片做出结构化裁决。
    RespondApproval {
        request_id: String,
        decision: AiApprovalDecision,
    },
    /// 手动触发上下文压缩并指定保留重点。
    CompactConversation {
        conversation_id: String,
        focus_instructions: String,
    },
    /// 回滚到指定消息轮次之前（同时恢复文件备份与对话分支）。
    RollbackCheckpoint {
        conversation_id: String,
        entry_id: String,
    },
    /// 终端内联命令生成（Ctrl+K）。
    InlineCommandAssist {
        request_id: String,
        prompt: String,
        host_label: String,
        platform: String,
    },
    /// 终端手动接管完成后恢复（或取消）挂起的 AI 运行。
    ResumeHandoff {
        run_id: String,
        terminal_output: Option<String>,
        cancelled: bool,
    },
    /// UI 回传从 `TerminalView` 采集到的命令块或屏幕输出。
    RespondTerminalContext {
        request_id: String,
        content: Option<String>,
    },
    /// UI 回传软件级动作执行结果。
    RespondAppAction {
        request_id: String,
        result: Result<String, String>,
    },
    /// 热切换执行模式（`manual` / `safe_read_auto` / `auto`）。
    SetExecutionMode { mode: String },
    /// 请求删除指定对话。
    DeleteConversation { conversation_id: String },
    /// 请求归档或取消归档指定对话。
    SetConversationArchived {
        conversation_id: String,
        archived: bool,
    },
    /// 请求重命名指定对话标题。
    RenameConversation {
        conversation_id: String,
        title: String,
    },
    /// 异步持久化 AI 设置（JSON 载荷由控制平面校验并落盘）。
    SaveSettingsJson { settings_json: String },
    /// 异步持久化全局记忆或指定主机长期记忆。
    SaveMemory {
        host_id: Option<String>,
        global_markdown: Option<String>,
        host_markdown: Option<String>,
    },
    /// 向供应商端点拉取可用模型列表。
    DiscoverModels {
        request_id: String,
        protocol: String,
        base_url: String,
        api_key: String,
    },
    /// 测试供应商端点连通性与延迟。
    TestConnection {
        request_id: String,
        protocol: String,
        base_url: String,
        api_key: String,
    },
    /// 一键安装外部 ACP Agent CLI 工具。
    InstallAcpAgent {
        agent_id: String,
        name: String,
        install_command: String,
    },
    /// 动态探测外部 ACP Agent 的实际可用模型与当前默认模型。
    ProbeAcpModels { agent_id: String },
    /// 停止单个托管后台任务（本机子进程、远端进程组或子 Agent）。
    CancelManagedTask { task_id: String },
    /// 清理界面中已结束的托管任务记录。
    DismissManagedTask { task_id: String },
    /// 将正在运行的托管任务转入后台继续流式输出，当前工具立即返回以便主对话继续。
    DetachManagedTask { task_id: String },
    /// 向正在运行的 AI 任务注入中途引导（Steer）补充指令。
    SteerRun { run_id: String, message: String },
}

/// 单轮推理结束后的统计指标摘要。
#[derive(Debug, Clone, Default)]
pub struct AiTurnMetricsDto {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub model_duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub today_tokens: u64,
    pub month_tokens: u64,
    pub context_breakdown: Option<AiContextBreakdownDto>,
}

/// 控制平面推送给 UI 的 AI 事件。
#[derive(Debug, Clone)]
pub enum AiAppEvent {
    RunStarted {
        run_id: String,
        conversation_id: String,
        title: String,
        /// 本轮用户消息落盘后的真实条目 ID，用于实时支持按轮次 Checkpoint 回滚。
        user_entry_id: String,
    },
    TextDelta {
        run_id: String,
        conversation_id: String,
        delta: String,
    },
    ReasoningDelta {
        run_id: String,
        conversation_id: String,
        delta: String,
    },
    TurnEnd {
        run_id: String,
        conversation_id: String,
        reasoning: String,
        text: String,
        tokens_per_sec: Option<f32>,
        metrics: AiTurnMetricsDto,
    },
    ToolCard(AiToolCardDto),
    ApprovalRequested(AiApprovalDto),
    TerminalContextRequested {
        request_id: String,
        tab_id: TabId,
        kind: AiTerminalContextKind,
    },
    /// 请求 UI 执行软件级动作（切换标签/面板、连接会话、操作转发或传输队列等）。
    AppActionRequested {
        request_id: String,
        domain: String,
        action: String,
        args_json: String,
    },
    RunFinished {
        run_id: String,
        conversation_id: String,
        error: Option<String>,
        /// 是否因用户取消或异常中断而结束（用于决定是否暂停自动出队）。
        cancelled: bool,
    },
    ModelsDiscovered {
        request_id: String,
        models: Vec<(String, Option<u32>)>,
        error: Option<String>,
    },
    ConnectionTested {
        request_id: String,
        ok: bool,
        detail: String,
    },
    PlanUpdated {
        run_id: String,
        conversation_id: String,
        plan: AiPlanDto,
    },
    FocusUpdated {
        run_id: String,
        focus: String,
    },
    AcpInstallStatus {
        agent_id: String,
        running: bool,
        ok: Option<bool>,
        detail: String,
    },
    AcpModelsProbed {
        agent_id: String,
        current_model: Option<String>,
        models: Vec<String>,
    },
    ManagedTaskStarted(AiManagedTaskDto),
    ManagedTaskOutput {
        task_id: String,
        chunk: String,
    },
    ManagedTaskFinished {
        task_id: String,
        status: AiManagedTaskStatus,
        exit_code: Option<i32>,
        summary: String,
    },
    ContextCompacted {
        conversation_id: String,
        summary: String,
    },
    ConversationRolledBack {
        conversation_id: String,
        restored_files: usize,
        summary: String,
    },
    InlineCommandResult {
        request_id: String,
        command: Option<String>,
        explanation: String,
        error: Option<String>,
    },
    TerminalHandoffRequested {
        run_id: String,
        host_label: String,
        command: String,
    },
}
