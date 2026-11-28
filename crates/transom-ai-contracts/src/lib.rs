//! Transom AI 契约层：配置、会话、策略、审计、检查点、记忆、技能与共享解析算法。
//!
//! 设计原理（Why）：
//! - 存储层只需要稳定的数据契约，不应因完整 Agent 运行时引入模型 SDK、网络客户端和工具执行器；
//! - 契约层保持纯 Rust、强类型和可序列化，供运行时、存储层与 UI 共享同一套业务语义；
//! - ACP 命令探测、选项提取与技能文件管理等轻量能力放在契约层，避免 UI 为少量纯逻辑
//!   引入完整模型 SDK、网络客户端和 Agent 执行器；
//! - 运行时边界继续由 `transom-ai` 提供兼容重导出，避免调用方感知内部拆分。

pub mod acp;
pub mod approval;
pub mod audit;
pub mod checkpoint;
pub mod choices;
pub mod config;
pub mod config_presets;
pub mod error;
pub mod exec_policy;
pub mod hooks;
pub mod ids;
pub mod memory;
pub mod policy;
pub mod policy_rules;
pub mod session;
pub mod skills;
pub mod thinking;

mod skills_builtin;
mod skills_io;

pub use acp::{acp_tools_bin_dir, augmented_path_env, is_acp_command_available};
pub use approval::{ApprovalGrantStore, ApprovalScope};
pub use audit::AiAuditEntry;
pub use checkpoint::{CheckpointManifest, FileBackupRecord, TurnCheckpoint};
pub use choices::{ExtractedChoices, extract_prompt_choices};
pub use config::{
    AcpAgentConfig, AiSettings, DEFAULT_COMMAND_TIMEOUT_SECS, DEFAULT_MAX_TOKENS,
    DEFAULT_MAX_TURNS, ModelConfig, ModelTarget, PRESETS, ProviderConfig, ProviderPreset,
    ProviderProtocol, RunningMessageBehavior, default_acp_presets, new_provider_id,
    provider_from_preset, provider_model_id,
};
pub use error::{AiError, ToolError};
pub use exec_policy::{
    ChainEvalResult, ExecPolicyAction, ExecPolicyRule, evaluate_command_chain,
    explain_command_impact, extract_command_prefix, split_shell_segments,
};
pub use hooks::{
    HookAction, HookContext, HookEvent, HookExecutor, HookMatcher, HookOutcome, HookRule,
    rule_matches,
};
pub use ids::{ApprovalRequestId, ConversationId, RunId, ToolCallId};
pub use memory::{
    MemoryProposal, MemorySnapshot, append_host_fact, read_memory_file_limited,
    resolve_sandbox_path,
};
pub use policy::{
    BlockingCommandHit, CommandPolicyContext, ExecutionMode, HostProtection, MAX_BULK_TARGETS,
    PolicyDecision, ShellCommandAssessment, ToolRisk, assess_shell_command, classify_shell_command,
    detect_blocking_command, evaluate_bulk_policy, evaluate_command_policy, evaluate_policy,
    format_blocking_command_hint,
};
pub use session::{
    ChatRole, ConversationSummary, SessionEntry, SessionEntryKind, SessionScope, SessionTree,
    TurnUsageRecord, extract_host_mentions,
};
pub use skills::{
    SkillDefinition, SkillSummary, builtin_skills, delete_custom_skill, extract_skill_command,
    import_skill_from_path, load_all_skills, load_skills_with_disabled, match_skill,
    parse_skill_markdown, read_skill_resource, sanitize_skill_id, save_custom_skill,
    summarize_enabled_skills,
};
pub use thinking::{
    ThinkingEffort, ThinkingSwitch, build_request_params, fallback_effort, request_params,
};
