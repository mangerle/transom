//! AI 助手主视图实体：装配顶栏、消息流、悬浮审批卡片、弹层菜单与输入卡片。

use std::sync::Arc;

use gpui_kit::base::v_flex;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use transom_core::{AiAppEvent, AppCommand, SecretStore};
use transom_store::AiStore;

use super::AiPanelEvent;
use super::approval::render_approval_card;
use super::attachments::{ComposerAttachment, should_convert_paste_to_doc};
use super::choice_card::{ChoiceCardState, init_choice_input};
use super::config_form::ConfigFormState;
use super::memory_drawer::MemoryFormState;
use super::plan_card::{PlanCardParams, render_plan_card};
use super::skills_drawer::SkillsDrawerState;
use super::slash_menu::register_slash_key_interceptor;
use super::state::{AiActivePopup, AiDrawerTab, AiPanelState};
use crate::palette::Palette;
use crate::theme::tokens::space;

/// AI 助手独立面板实体。
pub(crate) struct AiAssistantView {
    pub(super) state: AiPanelState,
    pub(super) ai_store: Arc<AiStore>,
    pub(super) secret_store: Arc<dyn SecretStore>,
    pub(super) command_tx: flume::Sender<AppCommand>,
    pub(super) composer_input: Entity<TextareaState>,
    pub(super) approval_feedback_input: Entity<InputState>,
    pub(super) config_form: ConfigFormState,
    pub(super) memory_form: MemoryFormState,
    pub(super) last_composer_len: usize,
    pub(super) slash_selected_idx: usize,
    pub(super) choice_card: ChoiceCardState,
    pub(super) msg_scroll_handle: ScrollHandle,
    pub(super) popover_scroll_handle: ScrollHandle,
    pub(super) active_nav_dot: Option<usize>,
    pub(super) copied_key: Option<String>,
    pub(super) _copy_reset_task: Option<Task<()>>,
    pub(super) _attachment_task: Option<Task<()>>,
    pub(super) _init_load_task: Option<Task<()>>,
    pub(super) _refresh_index_task: Option<Task<()>>,
    pub(super) skills_drawer: SkillsDrawerState,
    pub(super) policy_hooks_form: super::policy_hooks_drawer::PolicyHooksFormState,
    pub(super) model_picker: super::model_picker::ModelPickerState,
    pub(super) editing_title: bool,
    pub(super) title_input: Entity<InputState>,
    /// 用户主动向上滚动时设为 true，停止自动滚底。
    /// 点击"回到底部"按钮后重置为 false。
    pub(super) auto_scroll_locked: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AiPanelEvent> for AiAssistantView {}

impl AiAssistantView {
    /// 构造 AI 助手面板并异步加载配置、会话列表与全局记忆。
    pub(crate) fn new(
        ai_store: Arc<AiStore>,
        secret_store: Arc<dyn SecretStore>,
        command_tx: flume::Sender<AppCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut state = AiPanelState::default();
        if let Ok(settings) = ai_store.load_settings() {
            state.selected_model_id = settings.default_model.clone();
            state.selected_effort = settings.default_effort;
            state.execution_mode = settings.execution_mode;
            state.active_acp_agent_id = settings.active_acp_agent_id.clone();
            if let Some((prov, _)) = settings.resolve_model(&state.selected_model_id) {
                state.selected_protocol = prov.protocol;
            }
            state.settings = settings;
        }
        if state.selected_model_id.is_empty()
            && let Some((_, m)) = state.settings.models().next()
        {
            state.selected_model_id = m.id.clone();
        }
        if let Some(agent_id) = state.active_acp_agent_id.clone() {
            let _ = command_tx.try_send(AppCommand::Ai(Box::new(
                transom_core::AiCommand::ProbeAcpModels { agent_id },
            )));
        }

        let composer_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .submit_on_enter(true)
                .placeholder(rust_i18n::t!("ai.composer_placeholder").to_string())
        });
        let (config_form, form_subs) = ConfigFormState::new(
            &state.settings,
            &ai_store,
            secret_store.as_ref(),
            window,
            cx,
        );
        let memory_form = MemoryFormState::new(&state.memory.global_markdown, window, cx);
        let approval_feedback_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.approval_feedback_placeholder").to_string())
        });
        let (choice_custom_input, choice_sub) = init_choice_input(window, cx);
        let skills_drawer =
            SkillsDrawerState::new(&ai_store, &state.settings.disabled_skills, window, cx);
        let policy_hooks_form = super::policy_hooks_drawer::PolicyHooksFormState::new(window, cx);
        let model_picker = super::model_picker::ModelPickerState::new(window, cx);

        let sub = cx.subscribe_in(
            &composer_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { shift: false, .. } => {
                    if !this.try_apply_selected_slash(window, cx)
                        && !this.try_apply_selected_mention(window, cx)
                    {
                        this.send_from_composer(window, cx);
                    }
                }
                InputEvent::Change => {
                    this.slash_selected_idx = 0;
                    let current = this.composer_input.read(cx).value().to_string();
                    if should_convert_paste_to_doc(this.last_composer_len, &current) {
                        this.state.pasted_doc_seq += 1;
                        let title =
                            rust_i18n::t!("ai.pasted_doc_title", index = this.state.pasted_doc_seq)
                                .to_string();
                        let lines = current.lines().count().max(1);
                        let chars = current.chars().count();
                        this.state
                            .pending_attachments
                            .push(ComposerAttachment::PastedDoc {
                                id: uuid::Uuid::new_v4().to_string(),
                                title,
                                content: current,
                                lines,
                                chars,
                            });
                        this.last_composer_len = 0;
                        this.composer_input
                            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
                    } else {
                        this.last_composer_len = current.len();
                        if this.state.active_popup != AiActivePopup::None
                            && current.starts_with('/')
                        {
                            this.state.active_popup = AiActivePopup::None;
                        }
                    }
                    cx.notify();
                }
                _ => {}
            },
        );

        let key_sub = register_slash_key_interceptor(cx);

        let title_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.title_input_placeholder").to_string())
        });
        let title_sub = cx.subscribe_in(
            &title_input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.commit_title_edit(cx);
                }
                _ => {}
            },
        );

        let init_store = ai_store.clone();
        let bg = cx.background_executor().clone();
        let init_load_task = cx.spawn_in(window, async move |this, cx| {
            let loaded = bg
                .spawn(async move {
                    let conversations = init_store.list_all_conversations().unwrap_or_default();
                    let memory = init_store.load_memory(None).unwrap_or_default();
                    let totals = init_store.load_usage_totals();
                    (conversations, memory, totals)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let (conversations, memory, totals) = loaded;
                this.state.conversations = conversations;
                if this.state.memory.global_markdown.is_empty()
                    && !memory.global_markdown.is_empty()
                {
                    this.state.memory.global_markdown = memory.global_markdown;
                }
                this.state.metrics.set_stored_totals(totals);
                cx.notify();
            });
        });

        let mut subscriptions = vec![sub, key_sub, choice_sub, title_sub];
        subscriptions.extend(form_subs);

        Self {
            state,
            ai_store,
            secret_store,
            command_tx,
            composer_input,
            approval_feedback_input,
            config_form,
            memory_form,
            last_composer_len: 0,
            slash_selected_idx: 0,
            choice_card: ChoiceCardState::new(choice_custom_input),
            msg_scroll_handle: ScrollHandle::new(),
            popover_scroll_handle: ScrollHandle::new(),
            active_nav_dot: None,
            copied_key: None,
            _copy_reset_task: None,
            _attachment_task: None,
            _init_load_task: Some(init_load_task),
            _refresh_index_task: None,
            skills_drawer,
            policy_hooks_form,
            model_picker,
            editing_title: false,
            title_input,
            auto_scroll_locked: false,
            _subscriptions: subscriptions,
        }
    }

    /// 处理控制平面推送的 AI 事件。
    pub(crate) fn handle_app_event(&mut self, event: AiAppEvent, cx: &mut Context<Self>) {
        let refresh_index = matches!(
            event,
            AiAppEvent::RunStarted { .. } | AiAppEvent::RunFinished { .. }
        );
        let finished_cancelled = match &event {
            AiAppEvent::RunFinished { cancelled, .. } => Some(*cancelled),
            _ => None,
        };
        let reload_conv = match &event {
            AiAppEvent::ConversationRolledBack {
                conversation_id, ..
            } => Some(conversation_id.clone()),
            _ => None,
        };
        let subagent_wake = match &event {
            AiAppEvent::SubagentCompleted {
                conversation_id,
                subagent_id,
                title,
                ok,
                summary,
            } if self.state.active_conversation_id.as_deref() == Some(conversation_id.as_str()) => {
                let status = if *ok {
                    rust_i18n::t!("ai.subagent_done_ok").to_string()
                } else {
                    rust_i18n::t!("ai.subagent_done_err").to_string()
                };
                let id_note = if subagent_id.is_empty() {
                    String::new()
                } else {
                    format!(" (subagent_id: {subagent_id})")
                };
                Some(
                    rust_i18n::t!(
                        "ai.subagent_wake_notice",
                        title = title.as_str(),
                        id_note = id_note.as_str(),
                        status = status.as_str(),
                        summary = summary.as_str()
                    )
                    .to_string(),
                )
            }
            _ => None,
        };
        let models_discovered = matches!(event, AiAppEvent::ModelsDiscovered { .. });
        let should_scroll_bottom = matches!(
            event,
            AiAppEvent::RunStarted { .. }
                | AiAppEvent::TextDelta { .. }
                | AiAppEvent::ReasoningDelta { .. }
                | AiAppEvent::TurnEnd { .. }
                | AiAppEvent::ToolCard(_)
                | AiAppEvent::RunFinished { .. }
        );
        if matches!(event, AiAppEvent::RunStarted { .. }) {
            self.auto_scroll_locked = false;
            self.active_nav_dot = None;
        }
        self.state.apply_app_event(event);
        if should_scroll_bottom && !self.auto_scroll_locked {
            self.msg_scroll_handle.scroll_to_bottom();
        }
        if models_discovered {
            self.sync_picker_from_discovered(cx);
        }
        if let Some(cid) = reload_conv {
            self.select_conversation(&cid);
        }
        if refresh_index {
            let store = self.ai_store.clone();
            let bg = cx.background_executor().clone();
            self._refresh_index_task = Some(cx.spawn(async move |this, cx| {
                let conversations = bg
                    .spawn(async move { store.list_all_conversations().unwrap_or_default() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.state.conversations = conversations;
                    cx.notify();
                });
            }));
        }
        if let Some(cancelled) = finished_cancelled {
            self.maybe_drain_queue_after_finish(cancelled, cx);
        }
        if let Some(wake_prompt) = subagent_wake {
            if self.state.running_run_id.is_none() {
                self.send_prompt_text(wake_prompt, cx);
            } else {
                self.state
                    .queued_messages
                    .push(super::queue_card::QueuedPrompt {
                        id: uuid::Uuid::new_v4().to_string(),
                        prompt: wake_prompt,
                        attachments: Vec::new(),
                    });
            }
        }
        cx.notify();
    }

    fn render_chat_body(&self, p: &Palette, rem: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let mut container = v_flex()
            .flex_1()
            .w_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_message_stream(p, cx));

        let has_plan_steps = self
            .state
            .active_plan
            .as_ref()
            .is_some_and(|pl| !pl.steps.is_empty());
        if has_plan_steps && self.state.status_panel_expanded {
            let v_toggle = cx.entity();
            container = container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(
                render_plan_card(
                    PlanCardParams {
                        plan: self.state.active_plan.as_ref(),
                        expanded: true,
                        p,
                    },
                    move |_, cx| {
                        v_toggle.update(cx, |this, cx| {
                            this.state.status_panel_expanded = !this.state.status_panel_expanded;
                            cx.notify();
                        });
                    },
                ),
            ));
        }

        if let Some((_, host, cmd)) = &self.state.handoff_pending {
            let v_cont = cx.entity();
            let v_canc = cx.entity();
            container = container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(
                super::handoff_render::render_handoff_banner(
                    host,
                    cmd,
                    p,
                    move |_, cx| v_cont.update(cx, |this, cx| this.resume_handoff(false, cx)),
                    move |_, cx| v_canc.update(cx, |this, cx| this.resume_handoff(true, cx)),
                ),
            ));
        }

        if let Some(bar) = self.render_task_bar_section(p, cx) {
            container = container.child(bar);
        }

        if let Some(approval) = self.state.pending_approvals.first() {
            let req_id = approval.request_id.clone();
            let view_d = cx.entity();
            container = container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(
                render_approval_card(
                    approval,
                    &self.approval_feedback_input,
                    p,
                    rem,
                    move |decision, w, cx| {
                        let id = req_id.clone();
                        view_d.update(cx, |this, cx| this.respond_approval(&id, decision, w, cx));
                    },
                    cx,
                ),
            ));
        }

        if let Some(popover) = self.render_slash_floating_popover(p, cx) {
            container =
                container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(popover));
        } else if let Some(popover) = self.render_mention_floating_popover(p, cx) {
            container =
                container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(popover));
        } else if let Some(choice_card) = self.maybe_render_choice_card(p, cx) {
            container = container.child(
                div()
                    .px(rems(space::SM))
                    .pb(rems(space::XS))
                    .child(choice_card),
            );
        }

        if let Some(q_card) = super::queue_card::render_queued_messages_card(self, p, cx) {
            container = container.child(q_card);
        }

        if self.state.running_run_id.is_some()
            && let Some(focus_bar) = super::plan_card::render_running_focus_bar(
                self.state.active_focus.as_deref(),
                self.state
                    .active_plan
                    .as_ref()
                    .and_then(|pl| pl.focus.as_deref()),
                self.state.run_started_at.map(|t| t.elapsed().as_secs()),
                p,
            )
        {
            container = container.child(focus_bar);
        }

        container = container.child(self.render_chat_composer(p, cx));

        container.into_any_element()
    }
}

impl Render for AiAssistantView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let rem = window.rem_size();
        let header = self.render_header(&p, cx);

        let body = match self.state.drawer {
            AiDrawerTab::History => self.render_history_body(&p, cx),
            AiDrawerTab::Config => self.render_config_body(&p, cx),
            AiDrawerTab::Skills => self.render_skills_body(&p, cx),
            AiDrawerTab::Memory => self.render_memory_body(&p, cx),
            AiDrawerTab::Chat => self.render_chat_body(&p, rem, cx),
        };

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(p.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.state.active_popup != AiActivePopup::None {
                        this.state.active_popup = AiActivePopup::None;
                        cx.notify();
                    }
                }),
            )
            .child(header)
            .child(body)
    }
}
