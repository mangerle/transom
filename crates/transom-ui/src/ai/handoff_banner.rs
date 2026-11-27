//! 终端手动接管横幅渲染与 AI 面板扩展交互动作（压缩、回滚、Plan 模式切换、恢复接管）。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::ClipboardItem;
use gpui_kit::*;
use transom_ai::is_acp_command_available;
use transom_core::{AiCommand, AppCommand};
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon};

use super::AiPanelEvent;
use super::composer::{ComposerActions, ComposerParams, ExtraHostOption, render_composer};
use super::state::{AiActivePopup, AiDrawerTab};
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染终端手动接管等待横幅。
pub(super) fn render_handoff_banner(
    host_label: &str,
    command: &str,
    p: &Palette,
    on_continue: impl Fn(&mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    v_flex()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.primary)
        .child(
            h_flex()
                .gap(px(6.0))
                .items_center()
                .child(
                    div()
                        .text_color(p.primary)
                        .child(AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL)).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(rems(typography::CAPTION))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(
                            rust_i18n::t!("ai.handoff_banner_title", host = host_label).to_string(),
                        ),
                ),
        )
        .child(
            div()
                .p(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(p.background)
                .font_family("monospace")
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(command.to_owned()),
        )
        .child(
            h_flex()
                .justify_end()
                .gap(rems(space::XS))
                .child(
                    Button::new("ai-handoff-cancel")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(move |_, w, cx| on_cancel(w, cx)),
                )
                .child(
                    Button::new("ai-handoff-continue")
                        .primary()
                        .xsmall()
                        .label(rust_i18n::t!("ai.handoff_continue").to_string())
                        .on_click(move |_, w, cx| on_continue(w, cx)),
                ),
        )
        .into_any_element()
}

impl AiAssistantView {
    pub(super) fn compact_conversation(&mut self, focus: String, cx: &mut Context<Self>) {
        let Some(conv_id) = self.state.active_conversation_id.clone() else {
            return;
        };
        self.state.active_popup = AiActivePopup::None;
        let _ =
            self.command_tx
                .try_send(AppCommand::Ai(Box::new(AiCommand::CompactConversation {
                    conversation_id: conv_id,
                    focus_instructions: focus,
                })));
        cx.notify();
    }

    pub(super) fn rollback_checkpoint(&mut self, entry_id: String, cx: &mut Context<Self>) {
        let Some(conv_id) = self.state.active_conversation_id.clone() else {
            return;
        };
        if self.state.running_run_id.is_some() {
            return;
        }
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::RollbackCheckpoint {
                conversation_id: conv_id,
                entry_id,
            })));
        cx.notify();
    }

    pub(super) fn resume_handoff(&mut self, cancelled: bool, cx: &mut Context<Self>) {
        let Some((run_id, _, _)) = self.state.handoff_pending.take() else {
            return;
        };
        cx.emit(AiPanelEvent::ResumeHandoffRequested { run_id, cancelled });
        cx.notify();
    }

    pub(super) fn install_acp_agent(&mut self, agent_id: String, cx: &mut Context<Self>) {
        let agent = self
            .state
            .settings
            .acp_agents
            .iter()
            .find(|a| a.id == agent_id)
            .cloned()
            .or_else(|| {
                transom_ai::default_acp_presets()
                    .into_iter()
                    .find(|a| a.id == agent_id)
            });
        let Some(agent) = agent else {
            return;
        };
        let install_command = if !agent.install_command.trim().is_empty() {
            agent.install_command
        } else {
            transom_ai::default_acp_presets()
                .into_iter()
                .find(|p| p.id == agent.id)
                .map(|p| p.install_command)
                .unwrap_or_default()
        };
        if install_command.trim().is_empty() {
            self.state.acp_install_status = Some((
                agent_id,
                false,
                rust_i18n::t!("ai.acp_install_no_cmd").to_string(),
            ));
            cx.notify();
            return;
        }
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::InstallAcpAgent {
                agent_id,
                name: agent.name,
                install_command,
            })));
        cx.notify();
    }

    pub(super) fn handle_handoff_focus_request(&self, command: String, cx: &mut Context<Self>) {
        cx.emit(AiPanelEvent::FocusTerminalForHandoff { command });
    }

    pub(super) fn toggle_card_id(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.state.expanded_cards.remove(&id) {
            self.state.expanded_cards.insert(id);
        }
        cx.notify();
    }

    pub(super) fn copy_text_with_feedback(
        &mut self,
        key: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied_key = Some(key.clone());
        self._copy_reset_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.copied_key.as_deref() == Some(key.as_str()) {
                    this.copied_key = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn build_code_callbacks(
        &self,
        cx: &mut Context<Self>,
    ) -> super::markdown_blocks::CodeBlockCallbacks {
        let v_copy = cx.entity();
        let v_term = cx.entity();
        let v_snip = cx.entity();
        super::markdown_blocks::CodeBlockCallbacks {
            on_copy: std::rc::Rc::new(move |key, text, _, cx| {
                v_copy.update(cx, |this, cx| this.copy_text_with_feedback(key, text, cx));
            }),
            on_send_to_terminal: std::rc::Rc::new(move |command, _, cx| {
                v_term.update(cx, |_, cx| {
                    cx.emit(super::AiPanelEvent::SendToTerminalRequested { command });
                });
            }),
            on_save_snippet: std::rc::Rc::new(move |name, command, _, cx| {
                v_snip.update(cx, |this, cx| {
                    this.save_snippet_from_card(name, command, cx)
                });
            }),
        }
    }

    /// 外壳在活动标签切换或连接状态变更时单向注入会话摘要与已连接 SSH 列表。
    pub(crate) fn set_sessions(
        &mut self,
        session: Option<super::state::ActiveAiSessionInfo>,
        connected_sessions: Vec<super::state::ActiveAiSessionInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_changed = self.state.active_session != session;
        let list_changed = self.state.connected_sessions != connected_sessions;
        if !active_changed && !list_changed {
            return;
        }
        self.state.connected_sessions = connected_sessions;
        let primary_tab = session.as_ref().map(|s| s.tab_id);
        self.state.extra_scope_tabs.retain(|tid| {
            Some(*tid) != primary_tab
                && self
                    .state
                    .connected_sessions
                    .iter()
                    .any(|s| s.tab_id == *tid)
        });
        if !active_changed {
            cx.notify();
            return;
        }
        self.state.active_session = session;
        let scope = self.state.current_scope();
        let host_id = match &scope {
            transom_ai::SessionScope::Host(id) => Some(id.as_str()),
            transom_ai::SessionScope::Global => None,
        };
        if let Ok(mem) = self.ai_store.load_memory(host_id) {
            self.state.memory = mem;
            let g_text = self.state.memory.global_markdown.clone();
            let h_text = self.state.memory.host_markdown.clone();
            self.global_mem_input
                .update(cx, |i, cx| i.set_value(g_text, window, cx));
            self.host_mem_input
                .update(cx, |i, cx| i.set_value(h_text, window, cx));
        }
        cx.notify();
    }

    /// 外壳注入用于 `@` 提及补全的可用目标列表（SSH 会话、容器引擎与本机）。
    pub(crate) fn set_mention_candidates(
        &mut self,
        candidates: Vec<super::state::AiMentionCandidate>,
        cx: &mut Context<Self>,
    ) {
        if self.state.mention_candidates == candidates {
            return;
        }
        self.state.mention_candidates = candidates;
        cx.notify();
    }

    pub(super) fn render_chat_composer(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let active_acp = self.active_acp_agent();
        let base_model_label = if let Some(agent) = &active_acp {
            if !agent.default_model.trim().is_empty() {
                agent.default_model.clone()
            } else {
                transom_ai::default_acp_presets()
                    .into_iter()
                    .find(|p| p.id == agent.id)
                    .map(|p| p.default_model)
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| agent.name.clone())
            }
        } else {
            self.state
                .settings
                .resolve_model(&self.state.selected_model_id)
                .map(|(_, m)| m.display_label().to_owned())
                .unwrap_or_else(|| rust_i18n::t!("ai.no_model").to_string())
        };
        let model_label = if active_acp.is_none() {
            match self.state.selected_effort {
                Some(eff) => format!("{base_model_label} · {}", eff.as_str()),
                None => base_model_label,
            }
        } else {
            base_model_label
        };
        let session_badge = self.state.active_session.as_ref().map(|s| {
            format!(
                "{} · {}",
                s.host_name,
                if s.connected {
                    rust_i18n::t!("ai.connected").to_string()
                } else {
                    rust_i18n::t!("ai.disconnected").to_string()
                }
            )
        });
        let active_acp_id = self.state.active_acp_agent_id.clone();
        let acp_missing_warning = active_acp.as_ref().and_then(|agent| {
            (!is_acp_command_available(&agent.command))
                .then(|| (agent.id.clone(), agent.name.clone(), agent.command.clone()))
        });
        let acp_label = active_acp
            .map(|a| a.name)
            .unwrap_or_else(|| rust_i18n::t!("ai.acp_builtin_mode").to_string());

        let primary_tab = self.state.active_session.as_ref().map(|s| s.tab_id);
        let extra_host_options: Vec<ExtraHostOption> = if primary_tab.is_some() {
            self.state
                .connected_sessions
                .iter()
                .filter(|s| Some(s.tab_id) != primary_tab)
                .map(|s| ExtraHostOption {
                    tab_id: s.tab_id,
                    host_name: s.host_name.clone(),
                    selected: self.state.extra_scope_tabs.contains(&s.tab_id),
                })
                .collect()
        } else {
            Vec::new()
        };

        let view = cx.entity();
        let (
            v_stop,
            v_popup,
            v_extra,
            v_builtin,
            v_cfg,
            v_install,
            v_rm_att,
            v_paste,
            v_drop,
        ) = (
            view.clone(),
            view.clone(),
            view.clone(),
            view.clone(),
            view.clone(),
            view.clone(),
            view.clone(),
            view.clone(),
            view.clone(),
        );

        let running_tasks_count = self
            .state
            .managed_tasks
            .iter()
            .filter(|t| t.dto.status == transom_core::AiManagedTaskStatus::Running)
            .count();
        let v_pill_plan = view.clone();
        let v_pill_tasks = view.clone();
        let status_header = super::plan_card::render_status_pills_row(
            super::plan_card::StatusPillsParams {
                plan: self.state.active_plan.as_ref(),
                focus: self.state.active_focus.as_deref(),
                plan_expanded: self.state.status_panel_expanded,
                running_tasks_count,
                tasks_expanded: self.state.tasks_drawer_expanded,
                p,
            },
            move |_, cx| {
                v_pill_plan.update(cx, |this, cx| {
                    this.state.status_panel_expanded = !this.state.status_panel_expanded;
                    if this.state.status_panel_expanded {
                        this.state.tasks_drawer_expanded = false;
                    }
                    cx.notify();
                });
            },
            move |_, cx| {
                v_pill_tasks.update(cx, |this, cx| {
                    this.state.tasks_drawer_expanded = !this.state.tasks_drawer_expanded;
                    if this.state.tasks_drawer_expanded {
                        this.state.status_panel_expanded = false;
                    }
                    cx.notify();
                });
            },
        );

        render_composer(
            ComposerParams {
                input: &self.composer_input,
                is_running: self.state.running_run_id.is_some(),
                model_label,
                execution_mode: self.state.execution_mode,
                session_badge,
                acp_label,
                active_acp_id,
                acp_missing_warning,
                acp_install_status: self.state.acp_install_status.as_ref(),
                active_popup: self.state.active_popup,
                toolbar_popover: self.render_toolbar_popover(p, cx),
                pending_attachments: &self.state.pending_attachments,
                metrics: &self.state.metrics,
                context_window: self.active_context_window(),
                extra_host_options,
                status_header,
                p,
            },
            ComposerActions {
                on_send: Box::new(move |w, cx| {
                    view.update(cx, |this, cx| this.send_from_composer(w, cx));
                }),
                on_stop: Box::new(move |_, cx| {
                    v_stop.update(cx, |this, cx| this.cancel_running(cx));
                }),
                on_toggle_popup: Box::new(move |popup, _, cx| {
                    v_popup.update(cx, |this, cx| this.toggle_popup(popup, cx));
                }),
                on_toggle_extra_host: Box::new(move |tid, _, cx| {
                    v_extra.update(cx, |this, cx| this.toggle_extra_scope_tab(tid, cx));
                }),
                on_switch_builtin: Box::new(move |_, cx| {
                    v_builtin.update(cx, |this, cx| this.select_acp_agent(None, cx));
                }),
                on_open_config: Box::new(move |_, cx| {
                    v_cfg.update(cx, |this, cx| {
                        this.state.active_popup = AiActivePopup::None;
                        this.state.drawer = AiDrawerTab::Config;
                        cx.notify();
                    });
                }),
                on_install_acp: Box::new(move |id, _, cx| {
                    v_install.update(cx, |this, cx| this.install_acp_agent(id, cx));
                }),
                on_remove_attachment: Box::new(move |id, _, cx| {
                    v_rm_att.update(cx, |this, cx| this.remove_attachment(&id, cx));
                }),
                on_paste_item: Box::new(move |item, w, cx| {
                    v_paste.update(cx, |this, cx| this.handle_paste_item(item, w, cx))
                }),
                on_drop_paths: Box::new(move |paths, w, cx| {
                    v_drop.update(cx, |this, cx| this.handle_dropped_paths(paths, w, cx));
                }),
            },
        )
    }
}
