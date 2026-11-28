//! AI 助手主视图实体：装配顶栏、消息流、悬浮审批卡片、弹层菜单与输入卡片。

use std::sync::Arc;

use gpui_kit::base::v_flex;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use transom_core::{AiCommand, AppCommand, SecretStore};
use transom_store::AiStore;

use super::AiPanelEvent;
use super::approval::render_approval_card;
use super::attachments::{ComposerAttachment, should_convert_paste_to_doc};
use super::choice_card::{ChoiceCardState, init_choice_input};
use super::config_form::ConfigFormState;
use super::memory_drawer::MemoryFormState;
use super::message_stream::{MessageStreamEvent, MessageStreamView};
use super::plan_card::{PlanCardParams, render_plan_card};
use super::skills_drawer::{SkillsDrawerEvent, SkillsDrawerView};
use super::slash_menu::register_slash_key_interceptor;
use super::state::{AiActivePopup, AiDrawerTab, AiPanelState};
use crate::app::spawn_blocking_local;
use crate::palette::Palette;
use crate::theme::tokens::space;

/// AI 面板主视图持有的后台异步任务槽位（覆盖式取消旧任务）。
#[derive(Default)]
pub(super) struct AiPanelTasks {
    pub(super) copy_reset: Option<Task<()>>,
    pub(super) attachment: Option<Task<()>>,
    pub(super) _init_load: Option<Task<()>>,
    pub(super) refresh_index: Option<Task<()>>,
    pub(super) conversation_load: Option<Task<()>>,
    pub(super) memory_load: Option<Task<()>>,
}

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
    /// 用户按下 Esc 主动收起补全时的输入文本快照（输入变化后自动解除）。
    pub(super) completion_dismissed_input: Option<String>,
    pub(super) choice_card: ChoiceCardState,
    pub(super) popover_scroll_handle: ScrollHandle,
    pub(super) copied_key: Option<String>,
    pub(super) tasks: AiPanelTasks,
    pub(super) message_stream: Entity<MessageStreamView>,
    pub(super) skills_drawer: Entity<SkillsDrawerView>,
    pub(super) available_skills: Vec<transom_ai_contracts::SkillDefinition>,
    pub(super) policy_hooks_form: super::policy_hooks_drawer::PolicyHooksFormState,
    pub(super) model_picker: super::model_picker::ModelPickerState,
    pub(super) editing_title: bool,
    pub(super) title_input: Entity<InputState>,
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
        let settings_request_id = uuid::Uuid::new_v4().to_string();
        state.config.pending_settings_load = Some(settings_request_id.clone());
        if let Err(err) = command_tx.try_send(AppCommand::Ai(Box::new(AiCommand::LoadSettings {
            request_id: settings_request_id,
        }))) {
            state.config.pending_settings_load = None;
            state.view.status_error = Some(
                rust_i18n::t!("ai.load_config_request_failed", error = err.to_string()).to_string(),
            );
        }

        let composer_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .submit_on_enter(true)
                .placeholder(rust_i18n::t!("ai.composer_placeholder").to_string())
        });
        let (config_form, form_subs) = ConfigFormState::new(window, cx);
        let memory_form = MemoryFormState::new(&state.memory.global_markdown, window, cx);
        let approval_feedback_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.approval_feedback_placeholder").to_string())
        });
        let (choice_custom_input, choice_sub) = init_choice_input(window, cx);
        let skills_store = ai_store.clone();
        let skills_drawer = cx.new(|cx| SkillsDrawerView::new(skills_store, window, cx));
        let skills_sub =
            cx.subscribe(
                &skills_drawer,
                |this, _, event: &SkillsDrawerEvent, cx| match event {
                    SkillsDrawerEvent::ToggleSkillRequested(clean) => {
                        if let Some(pos) = this
                            .state
                            .config
                            .settings
                            .disabled_skills
                            .iter()
                            .position(|s| s.eq_ignore_ascii_case(clean))
                        {
                            this.state.config.settings.disabled_skills.remove(pos);
                        } else {
                            this.state
                                .config
                                .settings
                                .disabled_skills
                                .push(clean.clone());
                        }
                        this.persist_settings_reported(cx);
                    }
                    SkillsDrawerEvent::SkillsLoaded(skills) => {
                        this.available_skills = skills.clone();
                        cx.notify();
                    }
                },
            );
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
                    this.completion_dismissed_input = None;
                    let current = this.composer_input.read(cx).value().to_string();
                    if should_convert_paste_to_doc(this.last_composer_len, &current) {
                        this.state.composer.pasted_doc_seq += 1;
                        let title = rust_i18n::t!(
                            "ai.pasted_doc_title",
                            index = this.state.composer.pasted_doc_seq
                        )
                        .to_string();
                        let lines = current.lines().count().max(1);
                        let chars = current.chars().count();
                        this.state.composer.pending_attachments.push(
                            ComposerAttachment::PastedDoc {
                                id: uuid::Uuid::new_v4().to_string(),
                                title,
                                content: current,
                                lines,
                                chars,
                            },
                        );
                        this.last_composer_len = 0;
                        this.composer_input
                            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
                    } else {
                        this.last_composer_len = current.len();
                        if this.state.view.active_popup != AiActivePopup::None
                            && current.starts_with('/')
                        {
                            this.state.view.active_popup = AiActivePopup::None;
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
        let init_load_task = cx.spawn_in(window, async move |this, cx| {
            let loaded = spawn_blocking_local(move || {
                let conversations = init_store.list_all_conversations()?;
                let memory = init_store.load_memory(None)?;
                let totals = init_store.load_usage_totals()?;
                Ok::<_, transom_ai_contracts::AiError>((conversations, memory, totals))
            })
            .await;
            let loaded = loaded.unwrap_or_else(|| {
                Err(transom_ai_contracts::AiError::StorageFailed {
                    target: "ai_local_data_init".to_owned(),
                    reason: rust_i18n::t!("ai.background_task_failed").to_string(),
                })
            });
            if this
                .update(cx, |this, cx| {
                    match loaded {
                        Ok((conversations, memory, totals)) => {
                            this.state.conversation.conversations = conversations;
                            if this.state.memory.global_markdown.is_empty()
                                && !memory.global_markdown.is_empty()
                            {
                                this.state.memory.global_markdown = memory.global_markdown;
                            }
                            this.state.conversation.metrics.set_stored_totals(totals);
                        }
                        Err(err) => {
                            this.state.view.status_error = Some(
                                rust_i18n::t!("ai.load_local_data_failed", error = err.to_string())
                                    .to_string(),
                            );
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("AI 本地数据初始化视图已关闭，结果被丢弃");
            }
        });

        let message_stream = cx.new(|_| MessageStreamView::new());
        let stream_sub = cx.subscribe_in(
            &message_stream,
            window,
            |this, _, event: &MessageStreamEvent, w, cx| match event {
                MessageStreamEvent::ToggleCard(id) => this.toggle_card_id(id.clone(), cx),
                MessageStreamEvent::RollbackCheckpoint(entry_id) => {
                    this.rollback_checkpoint(entry_id.clone(), cx);
                }
                MessageStreamEvent::SendQuickPrompt(prompt) => {
                    let needs_target =
                        matches!(prompt.as_str(), "/inspect" | "/disk" | "/docker");
                    if this.state.conversation.active_session.is_none() && needs_target {
                        this.prefill_quick_prompt_with_mention(prompt, w, cx);
                    } else {
                        this.send_prompt_text(prompt.clone(), cx);
                    }
                }
                MessageStreamEvent::CopyText { key, text } => {
                    this.copy_text_with_feedback(key.clone(), text.clone(), cx);
                }
                MessageStreamEvent::SendToTerminal(command) => {
                    cx.emit(AiPanelEvent::SendToTerminalRequested {
                        command: command.clone(),
                    });
                }
                MessageStreamEvent::SaveSnippet { name, command } => {
                    cx.emit(AiPanelEvent::SaveSnippetRequested {
                        name: name.clone(),
                        command: command.clone(),
                    });
                }
                MessageStreamEvent::RevertRemoteFile {
                    host,
                    target,
                    backup,
                } => {
                    this.send_ai_command(
                        AiCommand::RevertRemoteFile {
                            host_label: host.clone(),
                            target_path: target.clone(),
                            backup_path: backup.clone(),
                            conversation_id: this.state.conversation.active_conversation_id.clone(),
                        },
                        cx,
                    );
                }
                MessageStreamEvent::JumpToTerminal(host_label) => {
                    cx.emit(AiPanelEvent::JumpToTerminal {
                        host_label: host_label.clone(),
                    });
                }
                MessageStreamEvent::SubagentAction { action, task_id } => {
                    match action.as_str() {
                        "stop" => {
                            this.send_ai_command(
                                AiCommand::CancelManagedTask {
                                    task_id: task_id.clone(),
                                },
                                cx,
                            );
                        }
                        "followup" => {
                            let msg = this.composer_input.read(cx).value().trim().to_owned();
                            if !msg.is_empty()
                                && this.send_ai_command(
                                    AiCommand::SteerRun {
                                        run_id: task_id.clone(),
                                        message: msg,
                                    },
                                    cx,
                                )
                            {
                                this.composer_input
                                    .update(cx, |i, cx| i.set_value(String::new(), w, cx));
                                this.last_composer_len = 0;
                            }
                        }
                        _ => {}
                    }
                    cx.notify();
                }
                MessageStreamEvent::LoadOlderMessages => {
                    this.state.conversation.render_window_blocks = this
                        .state
                        .conversation
                        .render_window_blocks
                        .saturating_add(160);
                    this.sync_message_stream(false, false, cx);
                }
            },
        );

        let mut subscriptions = vec![sub, key_sub, choice_sub, title_sub, skills_sub, stream_sub];
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
            completion_dismissed_input: None,
            choice_card: ChoiceCardState::new(choice_custom_input),
            popover_scroll_handle: ScrollHandle::new(),
            copied_key: None,
            tasks: AiPanelTasks {
                _init_load: Some(init_load_task),
                ..Default::default()
            },
            message_stream,
            skills_drawer,
            available_skills: Vec::new(),
            policy_hooks_form,
            model_picker,
            editing_title: false,
            title_input,
            _subscriptions: subscriptions,
        }
    }

    fn render_chat_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut container = v_flex()
            .flex_1()
            .w_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(self.message_stream.clone());

        let has_plan_steps = self.state.active_run_is_current()
            && self
                .state
                .run
                .active_plan
                .as_ref()
                .is_some_and(|pl| !pl.steps.is_empty());
        if has_plan_steps && self.state.view.status_panel_expanded {
            let v_toggle = cx.entity();
            container = container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(
                render_plan_card(
                    PlanCardParams {
                        plan: self.state.run.active_plan.as_ref(),
                        expanded: true,
                        p,
                    },
                    move |_, cx| {
                        v_toggle.update(cx, |this, cx| {
                            this.state.view.status_panel_expanded =
                                !this.state.view.status_panel_expanded;
                            cx.notify();
                        });
                    },
                ),
            ));
        }

        if self.state.active_run_is_current()
            && let Some((_, host, cmd)) = &self.state.run.handoff_pending
        {
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

        if self.state.active_run_is_current()
            && let Some(bar) = self.render_task_bar_section(p, cx)
        {
            container = container.child(bar);
        }

        let current_run_id = self.state.current_run_id();
        if let Some(approval) = self.state.run.pending_approvals.iter().find(|approval| {
            approval.owner_run_id == "mcp-external"
                || current_run_id == Some(approval.owner_run_id.as_str())
        }) {
            let req_id = approval.request_id.clone();
            let view_d = cx.entity();
            container = container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(
                render_approval_card(
                    approval,
                    &self.approval_feedback_input,
                    p,
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
        } else if let Some(popover) = self.render_target_session_popover(p, cx) {
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

        if self.state.has_active_run() && !self.state.active_run_is_current() {
            container = container.child(self.render_background_run_banner(p, cx));
        }

        if let Some(q_card) = super::queue_card::render_queued_messages_card(self, p, cx) {
            container = container.child(q_card);
        }

        if self.state.active_run_is_current()
            && let Some(focus_bar) = super::plan_card::render_running_focus_bar(
                self.state.run.active_focus.as_deref(),
                self.state
                    .run
                    .active_plan
                    .as_ref()
                    .and_then(|pl| pl.focus.as_deref()),
                self.state
                    .current_run_started_at()
                    .map(|time| time.elapsed().as_secs()),
                p,
            )
        {
            container = container.child(focus_bar);
        }

        container = container.child(self.render_chat_composer(p, cx));

        container.into_any_element()
    }

    fn render_background_run_banner(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::base::h_flex;
        use gpui_kit::component::Sizable as _;
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

        let bg_conv_id = self
            .state
            .background_run_conversation_id()
            .map(str::to_owned);
        let v_stop = cx.entity();
        let mut actions = h_flex().gap(px(4.0)).flex_shrink_0();
        if let Some(cid) = bg_conv_id {
            let v_jump = cx.entity();
            actions = actions.child(
                Button::new("ai-bg-run-jump")
                    .outline()
                    .xsmall()
                    .label(rust_i18n::t!("ai.bg_run_switch").to_string())
                    .on_click(move |_, _, cx| {
                        v_jump.update(cx, |this, cx| this.select_conversation(&cid, cx));
                    }),
            );
        }
        actions = actions.child(
            Button::new("ai-bg-run-stop")
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.bg_run_stop").to_string())
                .on_click(move |_, _, cx| {
                    v_stop.update(cx, |this, cx| this.cancel_background_run(cx));
                }),
        );

        div()
            .px(rems(space::SM))
            .pb(rems(space::XS))
            .child(
                h_flex()
                    .w_full()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(rems(space::XS))
                    .justify_between()
                    .items_center()
                    .rounded(p.radius.md)
                    .bg(p.surface.floating)
                    .border_1()
                    .border_color(p.warning.opacity(0.45))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.0))
                            .items_center()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(p.warning)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::STOP)).small()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(crate::theme::tokens::typography::CAPTION))
                                    .text_color(p.foreground)
                                    .child(rust_i18n::t!("ai.bg_run_banner_title").to_string()),
                            ),
                    )
                    .child(actions),
            )
            .into_any_element()
    }

    fn render_drawer_approval_banner(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::base::h_flex;
        use gpui_kit::component::Sizable as _;
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

        let count = self.state.run.pending_approvals.len();
        let view = cx.entity();
        h_flex()
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .justify_between()
            .items_center()
            .bg(p.warning.opacity(0.12))
            .border_b_1()
            .border_color(p.warning.opacity(0.4))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.0))
                    .items_center()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(p.warning)
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::LOCKED)).small()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(crate::theme::tokens::typography::CAPTION))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.foreground)
                            .child(
                                rust_i18n::t!("ai.pending_approval_banner", count = count)
                                    .to_string(),
                            ),
                    ),
            )
            .child(
                Button::new("ai-drawer-review-approval")
                    .primary()
                    .xsmall()
                    .label(rust_i18n::t!("ai.pending_approval_back").to_string())
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.state.view.drawer = AiDrawerTab::Chat;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }
}

impl Render for AiAssistantView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let header = self.render_header(&p, cx);

        let body = match self.state.view.drawer {
            AiDrawerTab::History => self.render_history_body(&p, cx),
            AiDrawerTab::Config => self.render_config_body(&p, cx),
            AiDrawerTab::Skills => self.skills_drawer.clone().into_any_element(),
            AiDrawerTab::Memory => self.render_memory_body(&p, cx),
            AiDrawerTab::Chat => self.render_chat_body(&p, cx),
        };

        let mut root = v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(p.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.state.view.active_popup != AiActivePopup::None {
                        this.state.view.active_popup = AiActivePopup::None;
                        cx.notify();
                    }
                }),
            )
            .child(header);
        if self.state.view.drawer != AiDrawerTab::Chat
            && !self.state.run.pending_approvals.is_empty()
        {
            root = root.child(self.render_drawer_approval_banner(&p, cx));
        }
        root.child(body)
    }
}
