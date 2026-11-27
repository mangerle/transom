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
    AiSettings, ConversationSummary, ExecutionMode, MemorySnapshot, ProviderProtocol, SessionScope,
    ThinkingEffort,
};
use transom_core::{AiAppEvent, AiApprovalDto, AiPlanDto, TabId};

pub(crate) use super::mention_menu::{AiMentionCandidate, AiMentionKind};

/// 一次 AI 事件处理产生、需要视图层继续同步控件的结果。
#[derive(Default)]
pub(crate) struct AiEventOutcome {
    /// 收到有效配置快照时返回，供表单与技能列表按同一份设置重建。
    pub(crate) settings_snapshot: Option<AiSettings>,
}

/// AI 助手面板状态组合根。
#[derive(Default)]
pub(crate) struct AiPanelState {
    pub(crate) config: AiConfigUiState,
    pub(crate) conversation: AiConversationUiState,
    pub(crate) run: AiRunUiState,
    pub(crate) composer: AiComposerUiState,
    pub(crate) view: AiViewUiState,
    pub(crate) memory: MemorySnapshot,
}

/// 设置、模型选择与 ACP 探测状态。
#[derive(Default)]
pub(crate) struct AiConfigUiState {
    pub(crate) settings: AiSettings,
    pub(crate) selected_model_id: String,
    pub(crate) selected_effort: Option<ThinkingEffort>,
    pub(crate) execution_mode: ExecutionMode,
    pub(crate) selected_protocol: ProviderProtocol,
    pub(crate) active_acp_agent_id: Option<String>,
    pub(crate) discovery_status: Option<String>,
    pub(crate) discovered_models: Vec<(String, Option<u32>)>,
    pub(crate) acp_install_status: Option<(String, bool, String)>,
    pub(crate) pending_settings_load: Option<String>,
    pub(crate) pending_settings_save: Option<String>,
}

/// 会话选择、消息流与统计状态。
#[derive(Default)]
pub(crate) struct AiConversationUiState {
    pub(crate) active_session: Option<ActiveAiSessionInfo>,
    pub(crate) connected_sessions: Vec<ActiveAiSessionInfo>,
    pub(crate) extra_scope_tabs: HashSet<TabId>,
    pub(crate) conversations: Vec<ConversationSummary>,
    pub(crate) show_archived_history: bool,
    pub(crate) active_conversation_id: Option<String>,
    pub(crate) active_conversation_title: Option<String>,
    pub(crate) items: Vec<UiChatItem>,
    pub(crate) expanded_cards: HashSet<String>,
    pub(crate) metrics: AiSessionMetrics,
}

/// 当前运行、审批、计划、任务与消息队列状态。
pub(crate) struct AiRunUiState {
    pub(crate) active_run: Option<ActiveAiRun>,
    pub(crate) pending_approvals: Vec<AiApprovalDto>,
    pub(crate) background_run_errors: HashMap<String, String>,
    pub(crate) active_plan: Option<AiPlanDto>,
    pub(crate) active_focus: Option<String>,
    pub(crate) handoff_pending: Option<(String, String, String)>,
    pub(crate) queued_messages: Vec<super::queue_card::QueuedPrompt>,
    pub(crate) queue_expanded: bool,
    pub(crate) queue_paused_by_cancel: bool,
    pub(crate) pending_interrupt_send: Option<super::queue_card::QueuedPrompt>,
    pub(crate) managed_tasks: Vec<super::task_bar::UiManagedTask>,
    pub(crate) tasks_drawer_expanded: bool,
    pub(crate) selected_task_id: Option<String>,
}

impl Default for AiRunUiState {
    fn default() -> Self {
        Self {
            active_run: None,
            pending_approvals: Vec::new(),
            background_run_errors: HashMap::new(),
            active_plan: None,
            active_focus: None,
            handoff_pending: None,
            queued_messages: Vec::new(),
            queue_expanded: true,
            queue_paused_by_cancel: false,
            pending_interrupt_send: None,
            managed_tasks: Vec::new(),
            tasks_drawer_expanded: false,
            selected_task_id: None,
        }
    }
}

/// 输入框附件、粘贴文档与提及候选状态。
#[derive(Default)]
pub(crate) struct AiComposerUiState {
    pub(crate) pending_attachments: Vec<ComposerAttachment>,
    pub(crate) pasted_doc_seq: usize,
    pub(crate) mention_candidates: Vec<AiMentionCandidate>,
}

/// 面板导航、弹层与错误提示状态。
pub(crate) struct AiViewUiState {
    pub(crate) drawer: AiDrawerTab,
    pub(crate) config_section: AiConfigSection,
    pub(crate) active_popup: AiActivePopup,
    pub(crate) status_panel_expanded: bool,
    pub(crate) status_error: Option<String>,
}

impl Default for AiViewUiState {
    fn default() -> Self {
        Self {
            drawer: AiDrawerTab::Chat,
            config_section: AiConfigSection::Models,
            active_popup: AiActivePopup::None,
            status_panel_expanded: true,
            status_error: None,
        }
    }
}

mod history;
mod run;
mod settings;
#[cfg(test)]
mod tests;

impl AiPanelState {
    /// 当前对话的作用域。
    pub(crate) fn current_scope(&self) -> SessionScope {
        match &self.conversation.active_session {
            Some(s) => SessionScope::Host(s.session_id.to_string()),
            None => SessionScope::Global,
        }
    }

    /// 消费来自控制平面的流式事件并更新界面消息流。
    pub(crate) fn apply_app_event(&mut self, event: AiAppEvent) -> AiEventOutcome {
        if !self.accepts_event(&event) {
            return AiEventOutcome::default();
        }
        let mut outcome = AiEventOutcome::default();
        match event {
            AiAppEvent::RunStarted {
                run_id,
                conversation_id,
                title,
                user_entry_id,
            } => {
                if self.run.active_run.is_none() {
                    self.begin_run(run_id.clone(), Some(conversation_id.clone()));
                } else {
                    self.bind_run(&run_id, &conversation_id);
                }
                self.run.background_run_errors.remove(&conversation_id);
                self.conversation.active_conversation_id = Some(conversation_id);
                if !title.trim().is_empty() {
                    self.conversation.active_conversation_title = Some(title);
                }
                self.view.status_error = None;
                self.run.active_plan = None;
                self.run.active_focus = None;
                self.view.status_panel_expanded = true;
                self.run.tasks_drawer_expanded = false;
                self.conversation.metrics.turns = self.conversation.metrics.turns.saturating_add(1);
                if !user_entry_id.trim().is_empty() {
                    let temp_id = format!("local-u-{run_id}");
                    for item in &mut self.conversation.items {
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
                }) = self.conversation.items.last_mut()
                {
                    text.push_str(&delta);
                } else {
                    self.conversation.items.push(UiChatItem::Reasoning {
                        id: format!("stream-r-{}", self.conversation.items.len()),
                        text: delta,
                        streaming: true,
                    });
                }
            }
            AiAppEvent::TextDelta { delta, .. } => {
                // 模型开始吐正文时收起上一条流式思考块
                if let Some(UiChatItem::Reasoning { streaming, .. }) =
                    self.conversation.items.last_mut()
                {
                    *streaming = false;
                }
                if let Some(UiChatItem::AssistantText {
                    text,
                    streaming: true,
                    ..
                }) = self.conversation.items.last_mut()
                {
                    text.push_str(&delta);
                } else {
                    self.conversation.items.push(UiChatItem::AssistantText {
                        id: format!("stream-t-{}", self.conversation.items.len()),
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
                self.conversation
                    .metrics
                    .record_turn(tokens_per_sec, &metrics);
                self.finalize_turn(reasoning, text, tokens_per_sec);
            }
            AiAppEvent::ToolCard(dto) => {
                if !dto.running {
                    self.conversation.metrics.steps =
                        self.conversation.metrics.steps.saturating_add(1);
                    self.conversation.metrics.tool_duration_ms = self
                        .conversation
                        .metrics
                        .tool_duration_ms
                        .saturating_add(dto.duration_ms);
                }
                self.push_tool_card(dto);
            }
            AiAppEvent::ApprovalRequested(dto) => {
                self.run.pending_approvals.push(dto);
            }
            AiAppEvent::RunFinished {
                run_id,
                conversation_id,
                error,
                ..
            } => {
                let visible = self.conversation_is_current(&conversation_id);
                self.finish_run(&run_id);
                self.run
                    .pending_approvals
                    .retain(|approval| approval.run_id != run_id);
                if self
                    .run
                    .handoff_pending
                    .as_ref()
                    .is_some_and(|(pending_run_id, _, _)| pending_run_id == &run_id)
                {
                    self.run.handoff_pending = None;
                }
                if visible {
                    self.run.active_focus = None;
                    if let Some(plan) = &mut self.run.active_plan {
                        plan.focus = None;
                    }
                    self.view.status_panel_expanded = false;
                    for item in &mut self.conversation.items {
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
                        self.view.status_error = Some(err);
                    }
                } else if let Some(err) = error {
                    self.run.background_run_errors.insert(conversation_id, err);
                }
            }
            AiAppEvent::TerminalContextRequested { .. } | AiAppEvent::AppActionRequested { .. } => {
            }
            AiAppEvent::SettingsSaved {
                request_id,
                revision: _,
                error,
            } => self.handle_settings_saved(request_id, error),
            AiAppEvent::SettingsSnapshot {
                request_id,
                settings_json,
                revision: _,
                error,
            } => {
                outcome.settings_snapshot =
                    self.handle_settings_snapshot(request_id, settings_json, error);
            }
            AiAppEvent::ModelsDiscovered { models, error, .. } => {
                if let Some(err) = error {
                    self.config.discovery_status = Some(err);
                } else {
                    self.config.discovered_models = models;
                    self.config.discovery_status = Some(format!(
                        "{} {}",
                        rust_i18n::t!("ai.discovered_models"),
                        self.config.discovered_models.len()
                    ));
                }
            }
            AiAppEvent::ConnectionTested { detail, .. } => {
                self.config.discovery_status = Some(detail);
            }
            AiAppEvent::PlanUpdated { plan, .. } => {
                if let Some(f) = &plan.focus {
                    self.run.active_focus = Some(f.clone());
                }
                self.run.active_plan = Some(plan);
            }
            AiAppEvent::FocusUpdated { focus, .. } => {
                self.run.active_focus = Some(focus);
            }
            AiAppEvent::AcpInstallStatus {
                agent_id,
                running,
                ok,
                detail,
            } => {
                self.config.acp_install_status = Some((agent_id, running, detail));
                if ok == Some(true) {
                    self.view.status_error = None;
                }
            }
            AiAppEvent::AcpModelsProbed {
                agent_id,
                current_model,
                models,
            } => {
                if !self
                    .config
                    .settings
                    .acp_agents
                    .iter()
                    .any(|a| a.id == agent_id)
                    && let Some(preset) = transom_ai::default_acp_presets()
                        .into_iter()
                        .find(|p| p.id == agent_id)
                {
                    self.config.settings.acp_agents.push(preset);
                }
                if let Some(agent) = self
                    .config
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
                    self.run
                        .pending_approvals
                        .retain(|approval| approval.run_id != task_id);
                }
            }
            AiAppEvent::TerminalHandoffRequested {
                run_id,
                host_label,
                command,
            } => {
                self.run.handoff_pending = Some((run_id, host_label, command));
            }
            AiAppEvent::ContextCompacted { summary, .. } => {
                self.run.active_focus = Some(summary);
            }
            AiAppEvent::ConversationRolledBack { summary, .. } => {
                self.run.active_focus = Some(summary);
            }
            AiAppEvent::InlineCommandResult { .. } | AiAppEvent::SubagentCompleted { .. } => {}
        }
        outcome
    }
}
