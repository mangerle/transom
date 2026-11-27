//! Transom 智能运维助手运行时：rig Agent 编排、协议适配、工具执行与安全审批。
//!
//! 配置、会话、策略、审计、检查点与记忆等稳定契约由 `transom-ai-contracts` 提供；
//! 本 crate 通过兼容重导出保持既有调用面，同时让存储层无需依赖完整运行时。

pub mod acp;
pub(crate) mod acp_handlers;
pub mod acp_parse;
pub mod acp_probe;
pub mod approval;
pub mod audit;
pub mod bm25;
pub mod checkpoint;
pub mod choices;
pub mod compaction;
pub mod config;
pub mod context_budget;
pub mod discovery;
pub mod error;
pub mod exec_policy;
pub mod hooks;
pub mod image;
pub mod mcp;
pub mod mcp_server;
pub(crate) mod mcp_tools;
pub mod memory;
pub mod output_store;
pub mod policy;
pub mod redact;
pub mod runner;
pub(crate) mod runner_gate;
pub mod session;
pub mod skills;
pub(crate) mod skills_builtin;
pub(crate) mod skills_io;
pub mod thinking;
pub mod tools;
pub mod working_state;

pub use acp::{AcpRunInput, is_acp_command_available, run_acp_turn};
pub use acp_probe::{AcpModelProbeResult, probe_acp_models};
pub use approval::{ApprovalGrantStore, ApprovalScope};
pub use audit::AiAuditEntry;
pub use bm25::{Bm25Document, Bm25Hit, Bm25Index};
pub use checkpoint::{CheckpointManifest, FileBackupRecord, TurnCheckpoint};
pub use choices::{ExtractedChoices, extract_prompt_choices};
pub use compaction::{
    PreambleExtra, SessionPromptContext, build_system_preamble, build_system_preamble_with_extra,
    strip_ansi, truncate_tool_output,
};
pub use config::{
    AcpAgentConfig, AiSettings, DEFAULT_COMMAND_TIMEOUT_SECS, DEFAULT_MAX_TOKENS,
    DEFAULT_MAX_TURNS, ModelConfig, ModelTarget, PRESETS, ProviderConfig, ProviderPreset,
    ProviderProtocol, RunningMessageBehavior, default_acp_presets, language_instruction,
    provider_from_preset,
};
pub use context_budget::{build_local_compaction_summary, compute_context_breakdown};
pub use discovery::{
    ConnectionTestResult, DiscoveredModel, discover_models, infer_context_window,
    parse_models_payload, test_provider_connection,
};
pub use error::{AiError, ToolError};
pub use exec_policy::{
    ChainEvalResult, ExecPolicyAction, ExecPolicyRule, evaluate_command_chain,
    explain_command_impact, extract_command_prefix, split_shell_segments,
};
pub use hooks::{
    HookAction, HookContext, HookEvent, HookExecutor, HookMatcher, HookOutcome, HookRule,
    rule_matches, send_webhook_json,
};
pub use image::PreparedImage;
pub use mcp::{McpServerConfig, McpToolDescriptor, McpTransport};
pub use mcp_server::{
    MCP_PROTOCOL_VERSION, McpServer, McpServerBackend, McpSessionInfo, serve_local_socket,
    serve_stream,
};
pub use memory::{
    MemoryProposal, MemorySnapshot, append_host_fact, read_memory_file_limited,
    resolve_sandbox_path,
};
pub use output_store::{ToolOutputStore, ToolOutputStorePool};
pub use policy::{
    BlockingCommandHit, CommandPolicyContext, ExecutionMode, HostProtection, MAX_BULK_TARGETS,
    PolicyDecision, ShellCommandAssessment, ToolRisk, assess_shell_command, classify_shell_command,
    detect_blocking_command, evaluate_bulk_policy, evaluate_command_policy, evaluate_policy,
    format_blocking_command_hint,
};
pub use redact::redact_secrets_for_model;
pub use runner::{
    AgentEvent, ApprovalRequest, EVENT_CHANNEL_CAPACITY, HandoffRequest, RunInput, TurnOutput,
    build_run_preamble, generate_inline_command, run, run_with_model, run_with_model_factory,
    summarize_for_compaction,
};
pub use session::{
    ChatRole, ConversationSummary, SessionEntry, SessionEntryKind, SessionScope, SessionTree,
    TurnUsageRecord, extract_host_mentions,
};
pub use skills::{
    SkillDefinition, SkillSummary, builtin_skills, delete_custom_skill, extract_skill_command,
    import_skill_from_path, load_all_skills, load_skills_with_disabled, match_skill,
    parse_skill_markdown, read_skill_resource, save_custom_skill, summarize_enabled_skills,
};
pub use thinking::{
    ThinkingEffort, ThinkingSwitch, build_request_params, fallback_effort, request_params,
};
pub use tools::{
    AppActionBridge, AppBridgeDomain, ApprovalSummary, BulkHostTarget, DelegateMode,
    DelegateTaskRequest, ExecCommandResult, ForwardExecutor, KnowledgeSearcher, LockedSession,
    MemoryWriter, RemoteExecutor, RemoteFsExecutor, RuntimeInspector, SkillLoader,
    SubagentModelTier, SubagentRunner, TerminalContextProvider, ToolCapabilities,
    ToolExecutionCard, ToolIoContext, ToolRuntimeContext, TransferDirection, assess_tool_call,
    plan::{PlanSnapshot, PlanStep, PlanStepStatus},
    with_io_guard,
};
pub use transom_ai_contracts::ids::{ApprovalRequestId, ConversationId, RunId, ToolCallId};
pub use working_state::{
    SessionWorkingState, aggressive_compact_history, is_context_overflow_error,
};
