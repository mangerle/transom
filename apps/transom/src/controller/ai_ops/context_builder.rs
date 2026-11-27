//! AI 运行期工具上下文（[`ToolRuntimeContext`]）装配器。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use transom_ai::{
    BulkHostTarget, ConversationSummary, LockedSession, MAX_BULK_TARGETS, PlanSnapshot,
    SessionEntry, SessionEntryKind, SessionPromptContext, SessionScope, ThinkingEffort,
    ToolOutputStore, ToolRuntimeContext,
};
use transom_core::{AiAppEvent, AiPromptRequest, AppEvent};
use uuid::Uuid;

use super::app_bridge::{self, StoreKnowledgeBridge, StoreMemoryBridge, StoreSkillLoader};
use super::bridge::{HostRuntimeBridge, SshRemoteExecutor, SshRemoteFsBridge, UiTerminalBridge};
use super::subagent;
use crate::controller::state::{ControllerCtx, ControllerState};

#[allow(clippy::too_many_arguments)]
pub(super) fn build_tool_context(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    req: &AiPromptRequest,
    scope: &SessionScope,
    conversation_id: &str,
    timeout_secs: &u64,
    cancel: CancellationToken,
    card_tx: mpsc::Sender<transom_ai::ToolExecutionCard>,
    plan_tx: Option<mpsc::Sender<PlanSnapshot>>,
    subagent_cfg: Option<(
        &transom_ai::ModelTarget,
        &transom_ai::MemorySnapshot,
        Option<String>,
    )>,
    current_turn_entry_id: Arc<parking_lot::RwLock<Option<String>>>,
) -> ToolRuntimeContext {
    let mut locked_session = None;
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
            tasks: Some(state.ai_tasks.clone()),
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
            pending: state.ai_terminal_reqs.clone(),
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
                    tasks: Some(state.ai_tasks.clone()),
                    host_label: extra.host_name.clone(),
                }),
            });
        }
    }

    let output_store = state
        .ai_output_stores
        .entry(conversation_id.to_owned())
        .or_insert_with(|| Arc::new(ToolOutputStore::new()))
        .clone();

    let command_timeout = Duration::from_secs((*timeout_secs).max(5));
    let mut bridge_deps = app_bridge::AppBridgeDeps::from_controller(
        state,
        ctx,
        req.tab_id,
        command_timeout,
        cancel.clone(),
    );
    bridge_deps.conversation_id = Some(conversation_id.to_owned());
    bridge_deps.current_turn_entry_id = current_turn_entry_id;
    let subagent_runner: Option<Arc<dyn transom_ai::SubagentRunner>> =
        subagent_cfg.map(|(target, memory, response_language)| {
            Arc::new(subagent::ControllerSubagentRunner {
                target: target.clone(),
                memory: memory.clone(),
                response_language,
                execution_mode: state.ai_execution_mode.clone(),
                app_bridge_deps: bridge_deps.clone(),
                tasks: state.ai_tasks.clone(),
                approvals: state.ai_approvals.clone(),
                event_tx: ctx.event_tx.clone(),
                parent_cancel: cancel.clone(),
            }) as Arc<dyn transom_ai::SubagentRunner>
        });
    let app_bridge: Arc<dyn transom_ai::AppActionBridge> =
        Arc::new(app_bridge::ControllerAppActionBridge::new(bridge_deps));

    ToolRuntimeContext {
        scope: scope.clone(),
        locked_session,
        bulk_hosts,
        output_store,
        command_timeout,
        cancel,
        card_tx,
        plan_tx,
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
        })),
    }
}

pub(super) async fn prepare_conversation(
    ctx: &ControllerCtx,
    req: &AiPromptRequest,
    scope: &SessionScope,
    model_name: &str,
) -> (String, String, Vec<SessionEntry>) {
    if let Some(conv_id) = &req.conversation_id
        && let Ok(tree) = ctx.ai_store.load_session_tree(conv_id)
    {
        let title = ctx
            .ai_store
            .load_index()
            .ok()
            .and_then(|list| list.into_iter().find(|i| &i.id == conv_id).map(|i| i.title))
            .unwrap_or_else(|| short_title(&req.prompt));
        let branch = tree.active_branch().into_iter().cloned().collect();
        return (conv_id.clone(), title, branch);
    }

    let conv_id = format!("chat-{}", Uuid::new_v4());
    let title = short_title(&req.prompt);
    let header = SessionEntry::new(
        None,
        SessionEntryKind::Header {
            title: title.clone(),
            scope: scope.clone(),
            model_id: model_name.to_owned(),
        },
    );
    let _ = ctx.ai_store.append_session_entry(&conv_id, &header);
    let _ = ctx.ai_store.upsert_conversation(ConversationSummary {
        id: conv_id.clone(),
        title: title.clone(),
        scope: scope.clone(),
        model_id: req.model_id.clone(),
        effort: req.effort.as_deref().and_then(ThinkingEffort::parse),
        updated_at: chrono::Utc::now().to_rfc3339(),
        active_leaf_id: Some(header.id.clone()),
        archived: false,
    });
    (conv_id, title, vec![header])
}

pub(super) trait StoreExt {
    fn store_append_if_ok(
        &self,
        conv_id: &str,
        entry: &SessionEntry,
    ) -> Result<(), transom_ai::AiError>;
}

impl StoreExt for ControllerCtx {
    fn store_append_if_ok(
        &self,
        conv_id: &str,
        entry: &SessionEntry,
    ) -> Result<(), transom_ai::AiError> {
        self.ai_store.append_session_entry(conv_id, entry)
    }
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
    let _ = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::RunFinished {
            run_id,
            conversation_id,
            error: Some(error),
            cancelled: false,
        })))
        .await;
}
