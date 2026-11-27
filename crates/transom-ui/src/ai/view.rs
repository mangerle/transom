//! AI 助手主视图实体：装配顶栏、消息流、悬浮审批卡片、弹层菜单与输入卡片。

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::base::v_flex;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::*;
use transom_ai::RunningMessageBehavior;
use transom_core::{AiAppEvent, AppCommand, SecretStore};
use transom_store::AiStore;

use super::AiPanelEvent;
use super::approval::render_approval_card;
use super::attachments::{ComposerAttachment, should_convert_paste_to_doc};
use super::choice_card::init_choice_input;
use super::config_drawer::{ConfigChoice, ConfigSelect, DraftModelRow};
use super::history_drawer::make_draft_model_row;
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
    pub(super) provider_name_input: Entity<InputState>,
    pub(super) api_key_input: Entity<InputState>,
    pub(super) base_url_input: Entity<InputState>,
    pub(super) draft_models: Vec<DraftModelRow>,
    pub(super) global_mem_input: Entity<InputState>,
    pub(super) host_mem_input: Entity<InputState>,
    pub(super) acp_cmd_input: Entity<InputState>,
    pub(super) approval_feedback_input: Entity<InputState>,
    pub(super) policy_pattern_input: Entity<InputState>,
    pub(super) hook_webhook_input: Entity<InputState>,
    pub(super) selected_acp_idx: usize,
    pub(super) last_composer_len: usize,
    pub(super) slash_selected_idx: usize,
    pub(super) choice_custom_input: Entity<InputState>,
    pub(super) choice_selected_idx: usize,
    pub(super) choice_multi_selected: HashSet<usize>,
    pub(super) choice_collapsed: bool,
    pub(super) dismissed_choice_id: Option<String>,
    pub(super) msg_scroll_handle: ScrollHandle,
    pub(super) popover_scroll_handle: ScrollHandle,
    pub(super) active_nav_dot: Option<usize>,
    pub(super) copied_key: Option<String>,
    pub(super) _copy_reset_task: Option<Task<()>>,
    pub(super) _attachment_task: Option<Task<()>>,
    pub(super) skills_drawer: SkillsDrawerState,
    pub(super) policy_hooks_form: super::policy_hooks_drawer::PolicyHooksFormState,
    pub(super) model_picker: super::model_picker::ModelPickerState,
    pub(super) lang_select: Entity<ConfigSelect>,
    pub(super) running_behavior_select: Entity<ConfigSelect>,
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
        state.conversations = ai_store.list_all_conversations().unwrap_or_default();
        state.memory = ai_store.load_memory(None).unwrap_or_default();
        state
            .metrics
            .set_stored_totals(ai_store.load_usage_totals());

        let composer_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .submit_on_enter(true)
                .placeholder(rust_i18n::t!("ai.composer_placeholder").to_string())
        });
        let provider_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.cfg_name_placeholder").to_string())
        });
        let api_key_input = cx.new(|cx| InputState::new(window, cx).placeholder("sk-..."));
        let base_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        let mut draft_models = Vec::new();
        if let Some(first_prov) = state.settings.providers.first() {
            let pname = first_prov.name.clone();
            let purl = first_prov.base_url.clone();
            provider_name_input.update(cx, |i, cx| i.set_value(pname, window, cx));
            base_url_input.update(cx, |i, cx| i.set_value(purl, window, cx));
            if let Ok(Some(saved_key)) =
                ai_store.load_provider_api_key(secret_store.as_ref(), &first_prov.id)
                && !saved_key.trim().is_empty()
            {
                api_key_input.update(cx, |i, cx| i.set_value(saved_key.as_str(), window, cx));
            }
            for m in &first_prov.models {
                draft_models.push(make_draft_model_row(
                    &m.display_name,
                    &m.name,
                    m.context_window,
                    m.efforts.first().copied(),
                    window,
                    cx,
                ));
            }
        }
        if draft_models.is_empty() {
            draft_models.push(make_draft_model_row(
                "",
                "",
                Some(128_000),
                None,
                window,
                cx,
            ));
        }
        let global_mem_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.global_memory_placeholder").to_string())
        });
        let host_mem_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.host_memory_placeholder").to_string())
        });
        let acp_cmd_input = cx.new(|cx| InputState::new(window, cx).placeholder("claude-code-acp"));
        let approval_feedback_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.approval_feedback_placeholder").to_string())
        });
        let policy_pattern_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.policy_pattern_placeholder").to_string())
        });
        let hook_webhook_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.hooks_webhook_placeholder").to_string())
        });
        let (choice_custom_input, choice_sub) = init_choice_input(window, cx);
        let skills_drawer =
            SkillsDrawerState::new(&ai_store, &state.settings.disabled_skills, window, cx);
        let policy_hooks_form = super::policy_hooks_drawer::PolicyHooksFormState::new(window, cx);
        let model_picker = super::model_picker::ModelPickerState::new(window, cx);

        let global_text = state.memory.global_markdown.clone();
        global_mem_input.update(cx, |input, cx| {
            input.set_value(global_text, window, cx);
        });

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

        let cur_lang = state.settings.response_language.clone();
        let lang_select = cx.new(|cx| {
            let items = vec![
                ConfigChoice::new("zh", rust_i18n::t!("ai.cfg_lang_zh").to_string()),
                ConfigChoice::new("en", rust_i18n::t!("ai.cfg_lang_en").to_string()),
                ConfigChoice::new("auto", rust_i18n::t!("ai.cfg_lang_auto").to_string()),
            ];
            let mut s = SelectState::new(SearchableVec::new(items), None, window, cx)
                .searchable(false);
            s.set_selected_value(&cur_lang, window, cx);
            s
        });
        let cur_beh_val = match state.settings.running_message_behavior {
            RunningMessageBehavior::Queue => "queue",
            RunningMessageBehavior::Steer => "steer",
            RunningMessageBehavior::Interrupt => "interrupt",
        };
        let running_behavior_select = cx.new(|cx| {
            let items = vec![
                ConfigChoice::new("queue", rust_i18n::t!("ai.running_msg_queue").to_string()),
                ConfigChoice::new("steer", rust_i18n::t!("ai.running_msg_steer").to_string()),
                ConfigChoice::new(
                    "interrupt",
                    rust_i18n::t!("ai.running_msg_interrupt").to_string(),
                ),
            ];
            let mut s = SelectState::new(SearchableVec::new(items), None, window, cx)
                .searchable(false);
            s.set_selected_value(&cur_beh_val.to_owned(), window, cx);
            s
        });

        let lang_sub = cx.subscribe_in(
            &lang_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ConfigChoice>>, _, cx| {
                if let SelectEvent::Confirm(Some(val)) = event {
                    this.state.settings.response_language = val.clone();
                    let _ = this.ai_store.save_settings(&this.state.settings);
                    cx.notify();
                }
            },
        );
        let beh_sub = cx.subscribe_in(
            &running_behavior_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ConfigChoice>>, _, cx| {
                if let SelectEvent::Confirm(Some(val)) = event {
                    let beh = match val.as_str() {
                        "steer" => RunningMessageBehavior::Steer,
                        "interrupt" => RunningMessageBehavior::Interrupt,
                        _ => RunningMessageBehavior::Queue,
                    };
                    this.set_running_message_behavior(beh, cx);
                }
            },
        );

        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("输入会话名称"));
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

        Self {
            state,
            ai_store,
            secret_store,
            command_tx,
            composer_input,
            provider_name_input,
            api_key_input,
            base_url_input,
            draft_models,
            global_mem_input,
            host_mem_input,
            acp_cmd_input,
            approval_feedback_input,
            policy_pattern_input,
            hook_webhook_input,
            selected_acp_idx: 0,
            last_composer_len: 0,
            slash_selected_idx: 0,
            choice_custom_input,
            choice_selected_idx: 0,
            choice_multi_selected: HashSet::new(),
            choice_collapsed: false,
            dismissed_choice_id: None,
            msg_scroll_handle: ScrollHandle::new(),
            popover_scroll_handle: ScrollHandle::new(),
            active_nav_dot: None,
            copied_key: None,
            _copy_reset_task: None,
            _attachment_task: None,
            skills_drawer,
            policy_hooks_form,
            model_picker,
            lang_select,
            running_behavior_select,
            editing_title: false,
            title_input,
            _subscriptions: vec![sub, key_sub, choice_sub, lang_sub, beh_sub, title_sub],
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
        let models_discovered = matches!(event, AiAppEvent::ModelsDiscovered { .. });
        self.state.apply_app_event(event);
        if models_discovered {
            self.sync_picker_from_discovered(cx);
        }
        if let Some(cid) = reload_conv {
            self.select_conversation(&cid);
        }
        if refresh_index {
            self.state.conversations = self.ai_store.list_all_conversations().unwrap_or_default();
        }
        if let Some(cancelled) = finished_cancelled {
            self.maybe_drain_queue_after_finish(cancelled, cx);
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

        let has_status_panel =
            self.state.active_plan.is_some() || self.state.active_focus.is_some();
        if has_status_panel && self.state.status_panel_expanded {
            let elapsed = self.state.run_started_at.map(|t| t.elapsed().as_secs());
            let v_toggle = cx.entity();
            container = container.child(div().px(rems(space::SM)).pb(rems(space::XS)).child(
                render_plan_card(
                    PlanCardParams {
                        plan: self.state.active_plan.as_ref(),
                        focus: self.state.active_focus.as_deref(),
                        elapsed_secs: elapsed,
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
                super::handoff_banner::render_handoff_banner(
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
