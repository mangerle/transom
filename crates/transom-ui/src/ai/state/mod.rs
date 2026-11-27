//! AI 助手面板的本地状态模型。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::attachments::ComposerAttachment;
use super::markdown_blocks::parse_markdown_segments;
use super::metrics_model::AiSessionMetrics;
pub(crate) use super::state_types::{
    ActiveAiRun, ActiveAiSessionInfo, AiActivePopup, AiConfigSection, AiDrawerTab, UiChatItem,
};
use transom_ai::{
    AiSettings, ConversationSummary, ExecutionMode, MemorySnapshot, ProviderProtocol, SessionEntry,
    SessionEntryKind, SessionScope, ThinkingEffort,
};
use transom_core::{AiAppEvent, AiApprovalDto, AiPlanDto, TabId};

pub(crate) use super::mention_menu::{AiMentionCandidate, AiMentionKind};

/// AI 助手面板运行状态。
pub(crate) struct AiPanelState {
    pub(crate) active_session: Option<ActiveAiSessionInfo>,
    pub(crate) connected_sessions: Vec<ActiveAiSessionInfo>,
    pub(crate) extra_scope_tabs: HashSet<TabId>,
    pub(crate) drawer: AiDrawerTab,
    pub(crate) config_section: AiConfigSection,
    pub(crate) settings: AiSettings,
    pub(crate) selected_model_id: String,
    pub(crate) selected_effort: Option<ThinkingEffort>,
    pub(crate) execution_mode: ExecutionMode,
    pub(crate) conversations: Vec<ConversationSummary>,
    pub(crate) show_archived_history: bool,
    pub(crate) active_conversation_id: Option<String>,
    pub(crate) active_conversation_title: Option<String>,
    pub(crate) selected_protocol: ProviderProtocol,
    pub(crate) items: Vec<UiChatItem>,
    pub(crate) expanded_cards: HashSet<String>,
    pub(crate) active_run: Option<ActiveAiRun>,
    pub(crate) pending_approvals: Vec<AiApprovalDto>,
    pub(crate) memory: MemorySnapshot,
    pub(crate) status_error: Option<String>,
    /// 后台对话运行失败时暂存的通知，切换回该对话后展示。
    pub(crate) background_run_errors: HashMap<String, String>,
    pub(crate) discovery_status: Option<String>,
    pub(crate) discovered_models: Vec<(String, Option<u32>)>,
    pub(crate) active_acp_agent_id: Option<String>,
    pub(crate) active_plan: Option<AiPlanDto>,
    pub(crate) active_focus: Option<String>,
    pub(crate) active_popup: AiActivePopup,
    pub(crate) status_panel_expanded: bool,
    pub(crate) pending_attachments: Vec<ComposerAttachment>,
    pub(crate) pasted_doc_seq: usize,
    pub(crate) acp_install_status: Option<(String, bool, String)>,
    pub(crate) metrics: AiSessionMetrics,
    pub(crate) managed_tasks: Vec<super::task_bar::UiManagedTask>,
    pub(crate) tasks_drawer_expanded: bool,
    pub(crate) selected_task_id: Option<String>,
    pub(crate) handoff_pending: Option<(String, String, String)>,
    pub(crate) queued_messages: Vec<super::queue_card::QueuedPrompt>,
    pub(crate) queue_expanded: bool,
    pub(crate) queue_paused_by_cancel: bool,
    pub(crate) pending_interrupt_send: Option<super::queue_card::QueuedPrompt>,
    pub(crate) mention_candidates: Vec<AiMentionCandidate>,
}

impl Default for AiPanelState {
    fn default() -> Self {
        Self {
            active_session: None,
            connected_sessions: Vec::new(),
            extra_scope_tabs: HashSet::new(),
            drawer: AiDrawerTab::Chat,
            config_section: AiConfigSection::Models,
            settings: AiSettings::default(),
            selected_model_id: String::new(),
            selected_effort: None,
            execution_mode: ExecutionMode::Manual,
            conversations: Vec::new(),
            show_archived_history: false,
            active_conversation_id: None,
            active_conversation_title: None,
            selected_protocol: ProviderProtocol::OpenAi,
            items: Vec::new(),
            expanded_cards: HashSet::new(),
            active_run: None,
            pending_approvals: Vec::new(),
            memory: MemorySnapshot::default(),
            status_error: None,
            background_run_errors: HashMap::new(),
            discovery_status: None,
            discovered_models: Vec::new(),
            active_acp_agent_id: None,
            active_plan: None,
            active_focus: None,
            active_popup: AiActivePopup::None,
            status_panel_expanded: true,
            pending_attachments: Vec::new(),
            pasted_doc_seq: 0,
            acp_install_status: None,
            metrics: AiSessionMetrics::default(),
            managed_tasks: Vec::new(),
            tasks_drawer_expanded: false,
            selected_task_id: None,
            handoff_pending: None,
            queued_messages: Vec::new(),
            queue_expanded: true,
            queue_paused_by_cancel: false,
            pending_interrupt_send: None,
            mention_candidates: Vec::new(),
        }
    }
}

mod run;
#[cfg(test)]
mod tests;

impl AiPanelState {
    /// 当前对话的作用域。
    pub(crate) fn current_scope(&self) -> SessionScope {
        match &self.active_session {
            Some(s) => SessionScope::Host(s.session_id.to_string()),
            None => SessionScope::Global,
        }
    }

    /// 把从 JSONL 加载的树状会话主分支还原为 UI 消息列表，并恢复该会话的推理指标。
    pub(crate) fn load_entries(&mut self, entries: Vec<SessionEntry>) {
        self.items.clear();
        self.metrics.restore_from_entries(&entries);
        for entry in entries {
            match entry.kind {
                SessionEntryKind::Message {
                    role,
                    text,
                    model,
                    tokens_per_sec,
                    ..
                } => match role {
                    transom_ai::ChatRole::User => {
                        self.items.push(UiChatItem::UserText {
                            id: entry.id,
                            text,
                            timestamp: Some(entry.timestamp),
                        });
                    }
                    transom_ai::ChatRole::Assistant => {
                        let cached_segments = Some(Arc::new(parse_markdown_segments(&text)));
                        self.items.push(UiChatItem::AssistantText {
                            id: entry.id,
                            text,
                            model,
                            tokens_per_sec,
                            streaming: false,
                            cached_segments,
                        });
                    }
                },
                SessionEntryKind::Reasoning { text } => {
                    self.items.push(UiChatItem::Reasoning {
                        id: entry.id,
                        text,
                        streaming: false,
                    });
                }
                SessionEntryKind::ToolRecord {
                    tool_name,
                    summary,
                    output,
                    ok,
                    duration_ms,
                    diff_preview,
                    revert_target_path,
                    revert_backup_path,
                    ..
                } => {
                    self.items.push(UiChatItem::ToolCard {
                        id: entry.id,
                        call_id: String::new(),
                        running: false,
                        tool_name,
                        host_label: self
                            .active_session
                            .as_ref()
                            .map(|s| s.host_name.clone())
                            .unwrap_or_else(|| rust_i18n::t!("ai.general_label").to_string()),
                        risk_label: String::new(),
                        summary,
                        output,
                        ok,
                        duration_ms,
                        diff_preview,
                        revert_target_path,
                        revert_backup_path,
                    });
                }
                SessionEntryKind::AuditNotice {
                    approved,
                    label,
                    host_label,
                } => {
                    self.items.push(UiChatItem::AuditNotice {
                        id: entry.id,
                        approved,
                        label,
                        host_label,
                    });
                }
                SessionEntryKind::Header { .. }
                | SessionEntryKind::Compaction { .. }
                | SessionEntryKind::Attachment { .. } => {}
            }
        }
    }

    /// 消费来自控制平面的流式事件并更新界面消息流。
    pub(crate) fn apply_app_event(&mut self, event: AiAppEvent) {
        if !self.accepts_event(&event) {
            return;
        }
        match event {
            AiAppEvent::RunStarted {
                run_id,
                conversation_id,
                title,
                user_entry_id,
            } => {
                if self.active_run.is_none() {
                    self.begin_run(run_id.clone(), Some(conversation_id.clone()));
                } else {
                    self.bind_run(&run_id, &conversation_id);
                }
                self.background_run_errors.remove(&conversation_id);
                self.active_conversation_id = Some(conversation_id);
                if !title.trim().is_empty() {
                    self.active_conversation_title = Some(title);
                }
                self.status_error = None;
                self.active_plan = None;
                self.active_focus = None;
                self.status_panel_expanded = true;
                self.tasks_drawer_expanded = false;
                self.metrics.turns = self.metrics.turns.saturating_add(1);
                if !user_entry_id.trim().is_empty() {
                    let temp_id = format!("local-u-{run_id}");
                    for item in &mut self.items {
                        if let UiChatItem::UserText { id, .. } = item
                            && *id == temp_id
                        {
                            *id = user_entry_id.clone();
                        }
                    }
                }
            }
            AiAppEvent::ReasoningDelta { delta, .. } => {
                if let Some(UiChatItem::Reasoning {
                    text,
                    streaming: true,
                    ..
                }) = self.items.last_mut()
                {
                    text.push_str(&delta);
                } else {
                    self.items.push(UiChatItem::Reasoning {
                        id: format!("stream-r-{}", self.items.len()),
                        text: delta,
                        streaming: true,
                    });
                }
            }
            AiAppEvent::TextDelta { delta, .. } => {
                // 模型开始吐正文时收起上一条流式思考块
                if let Some(UiChatItem::Reasoning { streaming, .. }) = self.items.last_mut() {
                    *streaming = false;
                }
                if let Some(UiChatItem::AssistantText {
                    text,
                    streaming: true,
                    ..
                }) = self.items.last_mut()
                {
                    text.push_str(&delta);
                } else {
                    self.items.push(UiChatItem::AssistantText {
                        id: format!("stream-t-{}", self.items.len()),
                        text: delta,
                        model: None,
                        tokens_per_sec: None,
                        streaming: true,
                        cached_segments: None,
                    });
                }
            }
            AiAppEvent::TurnEnd {
                reasoning,
                text,
                tokens_per_sec,
                metrics,
                ..
            } => {
                self.metrics.record_turn(tokens_per_sec, &metrics);
                self.finalize_turn(reasoning, text, tokens_per_sec);
            }
            AiAppEvent::ToolCard(dto) => {
                if !dto.running {
                    self.metrics.steps = self.metrics.steps.saturating_add(1);
                    self.metrics.tool_duration_ms = self
                        .metrics
                        .tool_duration_ms
                        .saturating_add(dto.duration_ms);
                }
                self.push_tool_card(dto);
            }
            AiAppEvent::ApprovalRequested(dto) => {
                self.pending_approvals.push(dto);
            }
            AiAppEvent::RunFinished {
                run_id,
                conversation_id,
                error,
                ..
            } => {
                let visible = self.conversation_is_current(&conversation_id);
                self.finish_run(&run_id);
                self.pending_approvals
                    .retain(|approval| approval.run_id != run_id);
                if self
                    .handoff_pending
                    .as_ref()
                    .is_some_and(|(pending_run_id, _, _)| pending_run_id == &run_id)
                {
                    self.handoff_pending = None;
                }
                if visible {
                    self.active_focus = None;
                    if let Some(plan) = &mut self.active_plan {
                        plan.focus = None;
                    }
                    self.status_panel_expanded = false;
                    for item in &mut self.items {
                        match item {
                            UiChatItem::Reasoning { streaming, .. } => {
                                *streaming = false;
                            }
                            UiChatItem::AssistantText {
                                text,
                                streaming,
                                cached_segments,
                                ..
                            } => {
                                *streaming = false;
                                if cached_segments.is_none() {
                                    *cached_segments =
                                        Some(Arc::new(parse_markdown_segments(text)));
                                }
                            }
                            UiChatItem::ToolCard { running, .. } => {
                                *running = false;
                            }
                            _ => {}
                        }
                    }
                    if let Some(err) = error {
                        self.status_error = Some(err);
                    }
                } else if let Some(err) = error {
                    self.background_run_errors.insert(conversation_id, err);
                }
            }
            AiAppEvent::TerminalContextRequested { .. } | AiAppEvent::AppActionRequested { .. } => {
            }
            AiAppEvent::ModelsDiscovered { models, error, .. } => {
                if let Some(err) = error {
                    self.discovery_status = Some(err);
                } else {
                    self.discovered_models = models;
                    self.discovery_status = Some(format!(
                        "{} {}",
                        rust_i18n::t!("ai.discovered_models"),
                        self.discovered_models.len()
                    ));
                }
            }
            AiAppEvent::ConnectionTested { detail, .. } => {
                self.discovery_status = Some(detail);
            }
            AiAppEvent::PlanUpdated { plan, .. } => {
                if let Some(f) = &plan.focus {
                    self.active_focus = Some(f.clone());
                }
                self.active_plan = Some(plan);
            }
            AiAppEvent::FocusUpdated { focus, .. } => {
                self.active_focus = Some(focus);
            }
            AiAppEvent::AcpInstallStatus {
                agent_id,
                running,
                ok,
                detail,
            } => {
                self.acp_install_status = Some((agent_id, running, detail));
                if ok == Some(true) {
                    self.status_error = None;
                }
            }
            AiAppEvent::AcpModelsProbed {
                agent_id,
                current_model,
                models,
            } => {
                if !self.settings.acp_agents.iter().any(|a| a.id == agent_id)
                    && let Some(preset) = transom_ai::default_acp_presets()
                        .into_iter()
                        .find(|p| p.id == agent_id)
                {
                    self.settings.acp_agents.push(preset);
                }
                if let Some(agent) = self
                    .settings
                    .acp_agents
                    .iter_mut()
                    .find(|a| a.id == agent_id)
                {
                    if !models.is_empty() {
                        agent.available_models = models;
                    }
                    if let Some(cur) = current_model
                        && !cur.trim().is_empty()
                    {
                        if !agent.available_models.contains(&cur) {
                            agent.available_models.insert(0, cur.clone());
                        }
                        agent.default_model = cur;
                    }
                }
            }
            other @ (AiAppEvent::ManagedTaskStarted(_)
            | AiAppEvent::ManagedTaskOutput { .. }
            | AiAppEvent::ManagedTaskFinished { .. }) => {
                let finished_task = match &other {
                    AiAppEvent::ManagedTaskFinished { task_id, .. } => Some(task_id.clone()),
                    _ => None,
                };
                super::task_bar::apply_managed_task_event(self, other);
                if let Some(task_id) = finished_task {
                    self.pending_approvals
                        .retain(|approval| approval.run_id != task_id);
                }
            }
            AiAppEvent::TerminalHandoffRequested {
                run_id,
                host_label,
                command,
            } => {
                self.handoff_pending = Some((run_id, host_label, command));
            }
            AiAppEvent::ContextCompacted { summary, .. } => {
                self.active_focus = Some(summary);
            }
            AiAppEvent::ConversationRolledBack { summary, .. } => {
                self.active_focus = Some(summary);
            }
            AiAppEvent::InlineCommandResult { .. } | AiAppEvent::SubagentCompleted { .. } => {}
        }
    }
}
