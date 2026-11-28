//! 控制平面 AI 命令分发与生命周期管理。

mod acp;
pub(in crate::controller) mod app_bridge;
mod bridge;
mod checkpoint_ops;
mod command_handlers;
mod compact;
mod config_service;
mod context_builder;
mod control;
mod credentials;
mod dispatch;
mod event_pump;
mod hooks_exec;
mod inline_assist;
mod mcp_audit;
mod mcp_bridge;
mod mcp_bridge_exec;
mod mcp_dispatch;
pub(in crate::controller) mod process;
mod remote_fs_probe;
mod run;
mod run_recorder;
mod runtime_bridge;
mod subagent;
pub(in crate::controller) mod task_runner;

use transom_ai::{SkillSummary, extract_skill_command};
use transom_core::{AiAppEvent, AiCommand, AppEvent};

use super::state::{ControllerCtx, ControllerState};
pub(in crate::controller) use config_service::AiConfigService;
use context_builder::{build_tool_context, prepare_conversation};
pub(in crate::controller) use control::AiControlState;
pub(in crate::controller) use dispatch::shutdown_ai_runs;
pub(in crate::controller) use mcp_bridge::{
    McpControllerSnapshot, handle_mcp_snapshot, spawn_mcp_server,
};

/// 向 UI 发送 AI 事件，并在事件总线关闭时记录统一告警。
pub(super) async fn send_ai_event(
    event_tx: &flume::Sender<AppEvent>,
    event: AiAppEvent,
    context: &str,
) {
    if let Err(err) = event_tx.send_async(AppEvent::Ai(Box::new(event))).await {
        log::warn!("发送 AI 事件失败（{context}）: {err}");
    }
}

/// 单次检索技能缓存并同时完成斜杠技能展开与可用技能摘要构造，避免重复读盘。
pub(super) fn load_and_expand_skills(
    store: &transom_store::AiStore,
    raw_prompt: &str,
    disabled_skills: &[String],
) -> (Option<String>, String, Vec<SkillSummary>) {
    let skills = store.load_skills_cached(disabled_skills);
    let (active_skill_prompt, effective_prompt) =
        if let Some((skill, rest)) = extract_skill_command(raw_prompt, &skills) {
            let user_text = if rest.is_empty() {
                skill.title.clone()
            } else {
                rest
            };
            (Some(skill.prompt_template.clone()), user_text)
        } else {
            (None, raw_prompt.to_owned())
        };
    let summaries = skills
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.to_summary())
        .collect();
    (active_skill_prompt, effective_prompt, summaries)
}

/// 处理来自 UI 的 AI 控制命令。
pub(in crate::controller) async fn handle_ai_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    cmd: AiCommand,
) {
    match cmd {
        AiCommand::CompactConversation { .. }
        | AiCommand::RollbackCheckpoint { .. }
        | AiCommand::RevertRemoteFile { .. }
        | AiCommand::DeleteConversation { .. }
        | AiCommand::SetConversationArchived { .. }
        | AiCommand::RenameConversation { .. }
        | AiCommand::SaveMemory { .. } => {
            command_handlers::handle_session_and_storage(state, ctx, cmd);
        }
        AiCommand::SaveSettingsJson { .. }
        | AiCommand::LoadSettings { .. }
        | AiCommand::DiscoverModels { .. }
        | AiCommand::TestConnection { .. }
        | AiCommand::InstallAcpAgent { .. }
        | AiCommand::ProbeAcpModels { .. } => {
            command_handlers::handle_config_and_probe(state, ctx, cmd).await;
        }
        AiCommand::SendPrompt(_)
        | AiCommand::InlineCommandAssist { .. }
        | AiCommand::CancelRun { .. }
        | AiCommand::RespondApproval { .. }
        | AiCommand::ResumeHandoff { .. }
        | AiCommand::RespondTerminalContext { .. }
        | AiCommand::RespondAppAction { .. }
        | AiCommand::CancelManagedTask { .. }
        | AiCommand::DismissManagedTask { .. }
        | AiCommand::DetachManagedTask { .. }
        | AiCommand::SteerRun { .. } => {
            command_handlers::handle_runtime_and_tasks(state, ctx, cmd).await;
        }
    }
}
