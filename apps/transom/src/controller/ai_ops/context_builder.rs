//! AI 运行期工具上下文（[`ToolRuntimeContext`]）装配器。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    AgentEvent, BulkHostTarget, ConversationSummary, LockedSession, MAX_BULK_TARGETS,
    ResolvedToolTarget, SessionEntry, SessionEntryKind, SessionPromptContext, SessionScope,
    ThinkingEffort, ToolRuntimeContext,
};
use transom_core::{AiAppEvent, AiPromptRequest, AppEvent};
use uuid::Uuid;

use super::app_bridge::{
    self, ControllerTargetResolver, StoreKnowledgeBridge, StoreMemoryBridge, StoreSkillLoader,
};
use super::bridge::{HostRuntimeBridge, SshRemoteExecutor, SshRemoteFsBridge, UiTerminalBridge};
use super::subagent;
use crate::controller::state::{ControllerCtx, ControllerState};
use transom_store::AiStore;

/// 子 Agent 继承父级上下文的配置对象。
pub(super) struct SubagentConfig<'a> {
    pub(super) parent_run_id: &'a str,
    pub(super) target: &'a transom_ai::ModelTarget,
    pub(super) fast_target: Option<&'a transom_ai::ModelTarget>,
    pub(super) memory: &'a transom_ai::MemorySnapshot,
    pub(super) response_language: Option<String>,
    pub(super) exec_policy_rules: &'a [transom_ai::ExecPolicyRule],
    pub(super) hooks: &'a [transom_ai::HookRule],
    pub(super) approval_scope: &'a transom_ai::ApprovalScope,
    pub(super) approval_grants: Arc<parking_lot::RwLock<transom_ai::ApprovalGrantStore>>,
    pub(super) execution_mode: Arc<parking_lot::RwLock<transom_ai::ExecutionMode>>,
}

/// 装配 [`ToolRuntimeContext`] 所需的参数对象。
pub(super) struct ToolContextBuildParams<'a> {
    pub(super) req: &'a AiPromptRequest,
    pub(super) scope: &'a SessionScope,
    pub(super) conversation_id: &'a str,
    pub(super) timeout_secs: u64,
    pub(super) cancel: CancellationToken,
    pub(super) events: mpsc::Sender<AgentEvent>,
    pub(super) subagent_cfg: Option<SubagentConfig<'a>>,
    pub(super) current_turn_entry_id: Arc<parking_lot::RwLock<Option<String>>>,
}

pub(super) fn build_tool_context(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    params: ToolContextBuildParams<'_>,
) -> ToolRuntimeContext {
    let ToolContextBuildParams {
        req,
        scope,
        conversation_id,
        timeout_secs,
        cancel,
        events,
        subagent_cfg,
        current_turn_entry_id,
    } = params;
    let mut locked_session = None;
    let mut default_target =
        ResolvedToolTarget::new(None, "通用会话", transom_ai::HostProtection::Normal);
    let mut remote_exec: Option<Arc<dyn transom_ai::RemoteExecutor>> = None;
    let mut remote_fs: Option<Arc<dyn transom_ai::RemoteFsExecutor>> = None;
    let mut terminal: Option<Arc<dyn transom_ai::TerminalContextProvider>> = None;
    let mut runtime_inspector: Option<Arc<dyn transom_ai::RuntimeInspector>> = None;
    let mut bulk_hosts: Vec<BulkHostTarget> = Vec::new();

    if let (Some(tab_id), Some(sid)) = (req.tab_id, req.session_id)
        && let Some(managed) = state.sessions.get(&tab_id)
        && let Some(remote) = managed.remote.clone()
    {
        let host_name = req
            .host_name
            .clone()
            .unwrap_or_else(|| "SSH 主机".to_owned());
        let login = req.login.clone().unwrap_or_default();
        let os_platform = req.platform.clone().unwrap_or_else(|| "linux".to_owned());
        default_target =
            ResolvedToolTarget::new(Some(sid.to_string()), host_name.clone(), req.protection);
        locked_session = Some(LockedSession {
            session_id: sid.to_string(),
            prompt_ctx: SessionPromptContext {
                host_name: host_name.clone(),
                login: login.clone(),
                os_platform,
                protection: req.protection,
            },
        });
        let exec = Arc::new(SshRemoteExecutor {
            remote: remote.clone(),
            tasks: Some(state.ai.managed_tasks.clone()),
            host_label: host_name.clone(),
        });
        bulk_hosts.push(BulkHostTarget {
            session_id: sid.to_string(),
            host_name,
            login,
            protection: req.protection,
            executor: exec.clone(),
        });
        remote_exec = Some(exec);
        remote_fs = Some(Arc::new(SshRemoteFsBridge {
            remote: remote.clone(),
            sftp: managed.sftp_client.clone(),
        }));
        terminal = Some(Arc::new(UiTerminalBridge {
            tab_id,
            event_tx: ctx.event_tx.clone(),
            pending: state.ai.terminal_reqs.clone(),
        }));
        runtime_inspector = Some(Arc::new(HostRuntimeBridge { remote }));
    }

    for extra in &req.extra_hosts {
        if bulk_hosts.len() >= MAX_BULK_TARGETS {
            break;
        }
        let sid_str = extra.session_id.to_string();
        if bulk_hosts.iter().any(|h| h.session_id == sid_str) {
            continue;
        }
        if let Some(managed) = state.sessions.get(&extra.tab_id)
            && let Some(remote) = managed.remote.clone()
        {
            bulk_hosts.push(BulkHostTarget {
                session_id: sid_str,
                host_name: extra.host_name.clone(),
                login: extra.login.clone(),
                protection: extra.protection,
                executor: Arc::new(SshRemoteExecutor {
                    remote,
                    tasks: Some(state.ai.managed_tasks.clone()),
                    host_label: extra.host_name.clone(),
                }),
            });
        }
    }

    let output_store = state
        .ai
        .output_stores
        .get_or_create(&transom_ai::ConversationId::from(conversation_id));

    let command_timeout = Duration::from_secs(timeout_secs.max(5));
    let mut bridge_deps = app_bridge::AppBridgeDeps::from_controller(
        state,
        ctx,
        req.tab_id,
        command_timeout,
        cancel.clone(),
    );
    bridge_deps.target.conversation_id = Some(conversation_id.to_owned());
    bridge_deps.target.current_turn_entry_id = current_turn_entry_id;
    let subagent_runner: Option<Arc<dyn transom_ai::SubagentRunner>> = subagent_cfg.map(|cfg| {
        Arc::new(subagent::ControllerSubagentRunner {
            parent_run_id: cfg.parent_run_id.to_owned(),
            target: cfg.target.clone(),
            fast_target: cfg.fast_target.cloned(),
            memory: cfg.memory.clone(),
            response_language: cfg.response_language,
            execution_mode: cfg.execution_mode.clone(),
            exec_policy_rules: cfg.exec_policy_rules.to_vec(),
            hooks: cfg.hooks.to_vec(),
            approval_scope: cfg.approval_scope.clone(),
            approval_grants: cfg.approval_grants,
            default_target: default_target.clone(),
            app_bridge_deps: bridge_deps.clone(),
            tasks: state.ai.managed_tasks.clone(),
            subagent_histories: state.ai.subagent_histories.clone(),
            approvals: state.ai.approvals.clone(),
            event_tx: ctx.event_tx.clone(),
            parent_cancel: cancel.clone(),
        }) as Arc<dyn transom_ai::SubagentRunner>
    });
    let target_resolver = Arc::new(ControllerTargetResolver::new(bridge_deps.clone()));
    let app_bridge: Arc<dyn transom_ai::AppActionBridge> =
        Arc::new(app_bridge::ControllerAppActionBridge::new(bridge_deps));

    ToolRuntimeContext {
        scope: scope.clone(),
        locked_session,
        default_target,
        bulk_hosts,
        output_store,
        command_timeout,
        cancel,
        events,
        snapshot_dir: Some(
            super::checkpoint_ops::store_ai_root(&ctx.ai_store)
                .join("checkpoints")
                .join("blobs"),
        ),
        caps: transom_ai::ToolCapabilities {
            remote_exec,
            remote_fs,
            terminal,
            runtime_inspector,
            forwarder: None,
            memory_writer: Some(Arc::new(StoreMemoryBridge {
                store: ctx.ai_store.clone(),
            })),
            knowledge_searcher: Some(Arc::new(StoreKnowledgeBridge {
                store: ctx.ai_store.clone(),
            })),
            app_bridge: Some(app_bridge),
            subagent_runner,
            skill_loader: Some(Arc::new(StoreSkillLoader {
                store: ctx.ai_store.clone(),
                ai_config: ctx.ai_config.clone(),
            })),
        },
        target_resolver: Some(target_resolver),
    }
}

/// 加载现有对话分支或创建新对话。
///
/// # Errors
/// 会话文件、索引读取失败，或新对话头与索引写入失败时返回
/// [`transom_ai::AiError::StorageFailed`]；调用方必须中止运行，不得静默切换到新对话。
pub(super) async fn prepare_conversation(
    ctx: &ControllerCtx,
    req: &AiPromptRequest,
    scope: &SessionScope,
    model_name: &str,
) -> Result<(String, String, Vec<SessionEntry>), transom_ai::AiError> {
    let store = ctx.ai_store.clone();
    let req = req.clone();
    let scope = scope.clone();
    let model_name = model_name.to_owned();
    tokio::task::spawn_blocking(move || {
        prepare_conversation_blocking(&store, &req, &scope, &model_name)
    })
    .await
    .map_err(|error| transom_ai::AiError::StorageFailed {
        target: "准备 AI 对话任务".to_owned(),
        reason: error.to_string(),
    })?
}

fn prepare_conversation_blocking(
    store: &AiStore,
    req: &AiPromptRequest,
    scope: &SessionScope,
    model_name: &str,
) -> Result<(String, String, Vec<SessionEntry>), transom_ai::AiError> {
    if let Some(conversation_id) = &req.conversation_id {
        let tree = store.load_session_tree(conversation_id).map_err(|err| {
            transom_ai::AiError::StorageFailed {
                target: format!("加载 AI 对话 {conversation_id}"),
                reason: err.to_string(),
            }
        })?;
        let title = store
            .load_index()
            .map_err(|err| transom_ai::AiError::StorageFailed {
                target: "加载 AI 对话索引".to_owned(),
                reason: err.to_string(),
            })?
            .into_iter()
            .find(|item| &item.id == conversation_id)
            .map(|item| item.title)
            .ok_or_else(|| transom_ai::AiError::StorageFailed {
                target: conversation_id.clone(),
                reason: "对话文件存在但索引条目缺失".to_owned(),
            })?;
        let branch = tree.active_branch().into_iter().cloned().collect();
        return Ok((conversation_id.clone(), title, branch));
    }

    let conversation_id = format!("chat-{}", Uuid::new_v4());
    let title = short_title(&req.prompt);
    let header = SessionEntry::new(
        None,
        SessionEntryKind::Header {
            title: title.clone(),
            scope: scope.clone(),
            model_id: model_name.to_owned(),
        },
    );
    store.append_session_entry(&conversation_id, &header)?;
    let summary = ConversationSummary {
        id: conversation_id.clone(),
        title: title.clone(),
        scope: scope.clone(),
        model_id: req.model_id.clone(),
        effort: req.effort.as_deref().and_then(ThinkingEffort::parse),
        updated_at: chrono::Utc::now().to_rfc3339(),
        active_leaf_id: Some(header.id.clone()),
        archived: false,
    };
    if let Err(err) = store.upsert_conversation(summary) {
        let cleanup = store.delete_conversation(&conversation_id);
        let reason = match cleanup {
            Ok(()) => err.to_string(),
            Err(cleanup_err) => {
                format!("{err}; 清理未索引对话文件失败: {cleanup_err}")
            }
        };
        return Err(transom_ai::AiError::StorageFailed {
            target: conversation_id,
            reason,
        });
    }
    Ok((conversation_id, title, vec![header]))
}

fn short_title(prompt: &str) -> String {
    let trimmed = prompt.trim();
    let mut chars = trimmed.chars();
    let head: String = chars.by_ref().take(36).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else if head.is_empty() {
        "新对话".to_owned()
    } else {
        head
    }
}

pub(super) async fn emit_run_error(
    ctx: &ControllerCtx,
    run_id: String,
    conversation_id: String,
    error: String,
) {
    if let Err(err) = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::RunFinished {
            run_id,
            conversation_id,
            error: Some(error),
            cancelled: false,
        })))
        .await
    {
        log::warn!("发送 AI 运行错误事件失败: {err}");
    }
}
