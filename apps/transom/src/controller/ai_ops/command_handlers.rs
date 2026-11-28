//! 控制平面 AI 命令按业务域拆分后的子分发器与存储异步任务辅助。

use transom_ai::RunId;
use transom_core::{AiAppEvent, AiCommand};

use super::{acp, checkpoint_ops, compact, dispatch, inline_assist, send_ai_event, task_runner};
use crate::controller::state::{ControllerCtx, ControllerState};

/// 在 `run_set` 与 `spawn_blocking` 中统一执行无需回传 UI 事件的本地存储写操作。
fn spawn_store_blocking(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    op_label: &'static str,
    job: impl FnOnce(std::sync::Arc<transom_store::AiStore>) + Send + 'static,
) {
    let store = ctx.ai_store.clone();
    state.ai.run_set.spawn(async move {
        if let Err(err) = tokio::task::spawn_blocking(move || job(store)).await {
            log::warn!("{op_label}的阻塞任务异常结束: {err}");
        }
    });
}

/// 分发会话与记忆持久化相关命令（归档、重命名、删除、压缩、回滚、文件还原、记忆保存）。
pub(super) fn handle_session_and_storage(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    cmd: AiCommand,
) {
    match cmd {
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
            spawn_store_blocking(state, ctx, "删除 AI 对话", move |store| {
                if let Err(err) = store.delete_conversation(&conversation_id) {
                    log::warn!("删除 AI 对话失败（{conversation_id}）: {err}");
                }
            });
        }
        AiCommand::SetConversationArchived {
            conversation_id,
            archived,
        } => {
            spawn_store_blocking(state, ctx, "归档 AI 对话", move |store| {
                if let Err(err) = store.set_conversation_archived(&conversation_id, archived) {
                    log::warn!("归档 AI 对话失败（{conversation_id}）: {err}");
                }
            });
        }
        AiCommand::RenameConversation {
            conversation_id,
            title,
        } => {
            spawn_store_blocking(state, ctx, "重命名 AI 对话", move |store| {
                if let Err(err) = store.rename_conversation(&conversation_id, &title) {
                    log::warn!("重命名 AI 对话失败（{conversation_id}）: {err}");
                }
            });
        }
        AiCommand::SaveMemory {
            host_id,
            global_markdown,
            host_markdown,
        } => {
            spawn_store_blocking(state, ctx, "保存 AI 记忆", move |store| {
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
            });
        }
        _ => {}
    }
}

/// 分发配置读写、模型发现、连接测试与 ACP 外部 Agent 安装/探测命令。
pub(super) async fn handle_config_and_probe(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    cmd: AiCommand,
) {
    match cmd {
        AiCommand::SaveSettingsJson {
            request_id,
            settings_json,
        } => {
            let result = ctx.ai_config.replace_json(&settings_json).await;
            let (revision, error) = match result {
                Ok(revision) => (revision, None),
                Err(err) => (ctx.ai_config.revision(), Some(err.to_string())),
            };
            send_ai_event(
                &ctx.event_tx,
                AiAppEvent::SettingsSaved {
                    request_id,
                    revision,
                    error,
                },
                "AI 配置保存结果",
            )
            .await;
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
            send_ai_event(
                &ctx.event_tx,
                AiAppEvent::SettingsSnapshot {
                    request_id,
                    settings_json,
                    revision,
                    error,
                },
                "AI 配置快照",
            )
            .await;
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
        _ => {}
    }
}

/// 分发运行控制、审批/交接响应、终端上下文回调与托管任务控制命令。
pub(super) async fn handle_runtime_and_tasks(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    cmd: AiCommand,
) {
    match cmd {
        AiCommand::SendPrompt(req) => dispatch::start_ai_run(state, ctx, *req).await,
        AiCommand::InlineCommandAssist {
            request_id,
            prompt,
            host_label,
            platform,
        } => {
            inline_assist::spawn_inline_assist(state, ctx, request_id, prompt, host_label, platform)
        }
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
                .remove(&transom_ai::ApprovalRequestId::from(request_id.as_str()))
                && sender.send(decision).is_err()
            {
                log::debug!("AI 审批接收方已取消，决策结果被丢弃");
            }
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
        AiCommand::SteerRun { run_id, message } if !message.trim().is_empty() => {
            if let Some(queue) = state.ai.steers.lock().get(&RunId::from(run_id.as_str())) {
                queue.lock().push_back(message);
            } else {
                state.ai.managed_tasks.steer_task(&run_id, message);
            }
        }
        _ => {}
    }
}
