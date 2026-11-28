//! AI 助手面板的本地状态模型。

use std::collections::{HashMap, HashSet};

use super::attachments::ComposerAttachment;
use super::metrics_model::AiSessionMetrics;
pub(crate) use super::state_types::{
    ActiveAiRun, ActiveAiSessionInfo, AiActivePopup, AiConfigSection, AiDrawerTab, AiTargetMode,
    UiChatImage, UiChatItem,
};
pub(in crate::app::ai) use stream::MessageStreamModel;
use transom_ai_contracts::{
    AiSettings, ConversationSummary, ExecutionMode, MemorySnapshot, ProviderProtocol, SessionScope,
    ThinkingEffort,
};
use transom_core::{AiAppEvent, AiApprovalDto, AiPlanDto, SessionId, TabId};

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
pub(crate) struct AiConversationUiState {
    /// 外壳注入的当前活动标签页会话快照（不代表最终锁定目标）。
    pub(crate) workspace_active_session: Option<ActiveAiSessionInfo>,
    /// 当前生效的主目标会话（由 `target_mode` 与可用会话共同解析）。
    pub(crate) active_session: Option<ActiveAiSessionInfo>,
    /// 目标会话选择模式（跟随标签页 / 仅全局 / 手动锁定）。
    pub(crate) target_mode: AiTargetMode,
    /// 等待连接完成后自动锁定的目标会话 ID。
    pub(crate) pending_pin_session_id: Option<SessionId>,
    pub(crate) connected_sessions: Vec<ActiveAiSessionInfo>,
    pub(crate) extra_scope_tabs: HashSet<TabId>,
    /// 未打开终端标签页、但被勾选加入后台静默协同的已保存 SSH 会话 ID。
    pub(crate) extra_scope_sessions: HashSet<SessionId>,
    pub(crate) conversations: Vec<ConversationSummary>,
    pub(crate) show_archived_history: bool,
    pub(crate) active_conversation_id: Option<String>,
    pub(crate) active_conversation_title: Option<String>,
    pub(crate) metrics: AiSessionMetrics,
    /// 会话切换序号，用于丢弃后台读取迟到的旧会话。
    pub(crate) load_seq: u64,
    /// 记忆刷新序号，用于丢弃活动主机切换前发起的旧读取。
    pub(crate) memory_load_seq: u64,
}

impl Default for AiConversationUiState {
    fn default() -> Self {
        Self {
            workspace_active_session: None,
            active_session: None,
            target_mode: AiTargetMode::FollowActiveTab,
            pending_pin_session_id: None,
            connected_sessions: Vec::new(),
            extra_scope_tabs: HashSet::new(),
            extra_scope_sessions: HashSet::new(),
            conversations: Vec::new(),
            show_archived_history: false,
            active_conversation_id: None,
            active_conversation_title: None,
            metrics: AiSessionMetrics::default(),
            load_seq: 0,
            memory_load_seq: 0,
        }
    }
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
pub(in crate::app::ai) mod stream;
#[cfg(test)]
mod tests;

impl AiConversationUiState {
    /// 根据当前选择模式、活动标签页与已连接会话列表，解析并同步有效主目标会话。
    ///
    /// 返回 `true` 表示最终生效的 `active_session` 发生了变化（需要刷新主机记忆）。
    pub(crate) fn sync_resolved_session(&mut self) -> bool {
        if let Some(pending_sid) = self.pending_pin_session_id
            && let Some(found) = self
                .connected_sessions
                .iter()
                .find(|s| s.session_id == pending_sid)
        {
            self.target_mode = AiTargetMode::PinnedTab(found.tab_id);
            self.pending_pin_session_id = None;
        }
        if let AiTargetMode::PinnedTab(tid) = self.target_mode
            && !self.connected_sessions.iter().any(|s| s.tab_id == tid)
        {
            self.target_mode = AiTargetMode::FollowActiveTab;
        }
        let next_active = match self.target_mode {
            AiTargetMode::FollowActiveTab => self.workspace_active_session.clone(),
            AiTargetMode::GlobalOnly => None,
            AiTargetMode::PinnedTab(tid) => self
                .connected_sessions
                .iter()
                .find(|s| s.tab_id == tid)
                .cloned(),
        };
        let primary_tab = next_active.as_ref().map(|s| s.tab_id);
        let primary_sid = next_active.as_ref().map(|s| s.session_id);
        self.extra_scope_tabs.retain(|tid| {
            Some(*tid) != primary_tab && self.connected_sessions.iter().any(|s| s.tab_id == *tid)
        });
        // 若已保存会话已在终端中打开，从静默协同集合移除，避免重复；未打开的继续保留
        self.extra_scope_sessions.retain(|sid| {
            Some(*sid) != primary_sid
                && !self.connected_sessions.iter().any(|s| s.session_id == *sid)
        });
        if self.active_session == next_active {
            return false;
        }
        self.active_session = next_active;
        true
    }
}

impl AiPanelState {
    /// 当前对话的作用域。
    pub(crate) fn current_scope(&self) -> SessionScope {
        match &self.conversation.active_session {
            Some(s) => SessionScope::Host(s.session_id.to_string()),
            None => SessionScope::Global,
        }
    }

    /// 当前选中的模型显示名称。
    pub(crate) fn active_model_name(&self) -> Option<String> {
        self.config
            .settings
            .resolve_model(&self.config.selected_model_id)
            .map(|(_, m)| m.name.clone())
    }

    /// 消费来自控制平面的流式事件并更新会话元状态与运行态。
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
                ..
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
            }
            AiAppEvent::ReasoningDelta { .. } | AiAppEvent::TextDelta { .. } => {}
            AiAppEvent::TurnEnd {
                tokens_per_sec,
                metrics,
                ..
            } => {
                self.conversation
                    .metrics
                    .record_turn(tokens_per_sec, &metrics);
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
                    && let Some(preset) = transom_ai_contracts::default_acp_presets()
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
