//! 控制平面 AI 命令分发与生命周期管理。

mod acp;
pub(in crate::controller) mod app_bridge;
mod bridge;
mod checkpoint_ops;
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

use transom_ai::{ApprovalRequestId, RunId, SkillSummary, extract_skill_command};
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
        AiCommand::SendPrompt(req) => dispatch::start_ai_run(state, ctx, *req).await,
        AiCommand::CancelRun { run_id } => {
            if let Some(token) = state.ai.runs.lock().get(&RunId::from(run_id.as_str())) {
                token.cancel();
            }
        }
        AiCommand::RespondApproval {
            request_id,
            decision,
        } => {
            if let Some(sender) = state
                .ai
                .approvals
                .lock()
                .remove(&ApprovalRequestId::from(request_id.as_str()))
                && sender.send(decision).is_err()
            {
                log::debug!("AI 审批接收方已取消，决策结果被丢弃");
            }
        }
        AiCommand::CompactConversation {
            conversation_id,
            focus_instructions,
        } => compact::spawn_compact_conversation(
            &mut state.ai.run_set,
            ctx.ai_store.clone(),
            ctx.ai_config.clone(),
            ctx.secret_store.clone(),
            ctx.event_tx.clone(),
            conversation_id,
            focus_instructions,
        ),
        AiCommand::RollbackCheckpoint {
            conversation_id,
            entry_id,
        } => checkpoint_ops::spawn_rollback_checkpoint(state, ctx, conversation_id, entry_id),
        AiCommand::RevertRemoteFile {
            host_label,
            target_path,
            backup_path,
            conversation_id,
        } => checkpoint_ops::spawn_revert_remote_file(
            state,
            ctx,
            host_label,
            target_path,
            backup_path,
            conversation_id,
        ),
        AiCommand::InlineCommandAssist {
            request_id,
            prompt,
            host_label,
            platform,
        } => {
            inline_assist::spawn_inline_assist(state, ctx, request_id, prompt, host_label, platform)
        }
        AiCommand::ResumeHandoff {
            run_id,
            terminal_output,
            cancelled,
        } => {
            if let Some(sender) = state.ai.handoffs.lock().remove(&run_id) {
                let payload = if cancelled {
                    Err(())
                } else {
                    Ok(terminal_output)
                };
                if sender.send(payload).is_err() {
                    log::debug!("终端交接请求方已取消，恢复结果被丢弃");
                }
            }
        }
        AiCommand::RespondTerminalContext {
            request_id,
            content,
        } => {
            if let Some(sender) = state.ai.terminal_reqs.lock().remove(&request_id)
                && sender.send(content).is_err()
            {
                log::debug!("终端上下文请求方已取消，响应内容被丢弃");
            }
        }
        AiCommand::RespondAppAction { request_id, result } => {
            if let Some(sender) = state.ai.app_action_reqs.lock().remove(&request_id)
                && sender.send(result).is_err()
            {
                log::debug!("软件动作请求方已取消，执行结果被丢弃");
            }
        }
        AiCommand::DeleteConversation { conversation_id } => {
            state
                .ai
                .output_stores
                .remove(&transom_ai::ConversationId::from(conversation_id.as_str()));
            state
                .ai
                .approval_grants
                .write()
                .remove_namespace(&conversation_id);
            let store = ctx.ai_store.clone();
            state.ai.run_set.spawn(async move {
                if let Err(err) = tokio::task::spawn_blocking(move || {
                    if let Err(err) = store.delete_conversation(&conversation_id) {
                        log::warn!("删除 AI 对话失败（{conversation_id}）: {err}");
                    }
                })
                .await
                {
                    log::warn!("删除 AI 对话的阻塞任务异常结束: {err}");
                }
            });
        }
        AiCommand::SetConversationArchived {
            conversation_id,
            archived,
        } => {
            let store = ctx.ai_store.clone();
            state.ai.run_set.spawn(async move {
                if let Err(err) = tokio::task::spawn_blocking(move || {
                    if let Err(err) = store.set_conversation_archived(&conversation_id, archived) {
                        log::warn!("归档 AI 对话失败（{conversation_id}）: {err}");
                    }
                })
                .await
                {
                    log::warn!("归档 AI 对话的阻塞任务异常结束: {err}");
                }
            });
        }
        AiCommand::RenameConversation {
            conversation_id,
            title,
        } => {
            let store = ctx.ai_store.clone();
            state.ai.run_set.spawn(async move {
                if let Err(err) = tokio::task::spawn_blocking(move || {
                    if let Err(err) = store.rename_conversation(&conversation_id, &title) {
                        log::warn!("重命名 AI 对话失败（{conversation_id}）: {err}");
                    }
                })
                .await
                {
                    log::warn!("重命名 AI 对话的阻塞任务异常结束: {err}");
                }
            });
        }
        AiCommand::SaveSettingsJson {
            request_id,
            settings_json,
        } => {
            let result = ctx.ai_config.replace_json(&settings_json).await;
            let (revision, error) = match result {
                Ok(revision) => (revision, None),
                Err(err) => (ctx.ai_config.revision(), Some(err.to_string())),
            };
            if let Err(err) = ctx
                .event_tx
                .send_async(transom_core::AppEvent::Ai(Box::new(
                    transom_core::AiAppEvent::SettingsSaved {
                        request_id,
                        revision,
                        error,
                    },
                )))
                .await
            {
                log::warn!("发送 AI 配置保存结果失败: {err}");
            }
        }
        AiCommand::LoadSettings { request_id } => {
            let (settings_json, revision, error) = match ctx.ai_config.snapshot_json() {
                Ok(snapshot) => (
                    Some(snapshot.settings_json),
                    snapshot.revision,
                    snapshot.load_error,
                ),
                Err(err) => (None, 0, Some(err.to_string())),
            };
            if let Err(err) = ctx
                .event_tx
                .send_async(transom_core::AppEvent::Ai(Box::new(
                    transom_core::AiAppEvent::SettingsSnapshot {
                        request_id,
                        settings_json,
                        revision,
                        error,
                    },
                )))
                .await
            {
                log::warn!("发送 AI 配置快照失败: {err}");
            }
        }
        AiCommand::SaveMemory {
            host_id,
            global_markdown,
            host_markdown,
        } => {
            let store = ctx.ai_store.clone();
            state.ai.run_set.spawn(async move {
                if let Err(err) = tokio::task::spawn_blocking(move || {
                    if let Some(global) = global_markdown
                        && let Err(err) = store.save_global_memory(&global)
                    {
                        log::warn!("异步保存全局 AI 记忆失败: {err}");
                    }
                    if let (Some(hid), Some(host_md)) = (host_id, host_markdown)
                        && let Err(err) = store.save_host_memory(&hid, &host_md)
                    {
                        log::warn!("异步保存主机 AI 记忆失败（{hid}）: {err}");
                    }
                })
                .await
                {
                    log::warn!("保存 AI 记忆的阻塞任务异常结束: {err}");
                }
            });
        }
        AiCommand::DiscoverModels {
            request_id,
            provider_id,
            protocol,
            base_url,
            api_key,
        } => acp::spawn_discover_models(
            state,
            ctx,
            acp::ConnectionProbeParams {
                request_id,
                provider_id,
                protocol,
                base_url,
                api_key,
            },
        ),
        AiCommand::TestConnection {
            request_id,
            provider_id,
            protocol,
            base_url,
            api_key,
        } => acp::spawn_test_connection(
            state,
            ctx,
            acp::ConnectionProbeParams {
                request_id,
                provider_id,
                protocol,
                base_url,
                api_key,
            },
        ),
        AiCommand::InstallAcpAgent {
            agent_id,
            name,
            install_command,
        } => acp::spawn_install_acp_agent(state, ctx, agent_id, name, install_command),
        AiCommand::ProbeAcpModels { agent_id } => acp::spawn_probe_acp_models(state, ctx, agent_id),
        AiCommand::CancelManagedTask { task_id } => {
            if let Some((remote, pid)) = state.ai.managed_tasks.cancel_task(&task_id) {
                state
                    .ai
                    .run_set
                    .spawn(task_runner::kill_remote_pid(remote, pid));
            }
        }
        AiCommand::DismissManagedTask { .. } => {}
        AiCommand::DetachManagedTask { task_id } => {
            state.ai.managed_tasks.detach_task(&task_id);
        }
        AiCommand::SteerRun { run_id, message } => {
            if !message.trim().is_empty() {
                if let Some(queue) = state.ai.steers.lock().get(&RunId::from(run_id.as_str())) {
                    queue.lock().push_back(message);
                } else {
                    state.ai.managed_tasks.steer_task(&run_id, message);
                }
            }
        }
    }
}
