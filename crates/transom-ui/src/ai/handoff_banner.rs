//! AI 面板扩展交互动作（压缩、回滚、Plan 模式切换、恢复接管）。

use gpui_kit::gpui::ClipboardItem;
use gpui_kit::*;
use transom_ai_contracts::is_acp_command_available;
use transom_core::AiCommand;

use super::AiPanelEvent;
use super::composer::{ComposerActions, ComposerParams, ExtraHostOption, render_composer};
use super::state::{AiActivePopup, AiDrawerTab};
use super::view::AiAssistantView;
use crate::app::spawn_blocking_local;
use crate::palette::Palette;

impl AiAssistantView {
    pub(super) fn compact_conversation(&mut self, focus: String, cx: &mut Context<Self>) {
        let Some(conv_id) = self.state.conversation.active_conversation_id.clone() else {
            return;
        };
        self.state.view.active_popup = AiActivePopup::None;
        self.send_ai_command(
            AiCommand::CompactConversation {
                conversation_id: conv_id,
                focus_instructions: focus,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn rollback_checkpoint(&mut self, entry_id: String, cx: &mut Context<Self>) {
        let Some(conv_id) = self.state.conversation.active_conversation_id.clone() else {
            return;
        };
        if self.state.has_active_run() {
            return;
        }
        self.send_ai_command(
            AiCommand::RollbackCheckpoint {
                conversation_id: conv_id,
                entry_id,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn resume_handoff(&mut self, cancelled: bool, cx: &mut Context<Self>) {
        let Some((run_id, _, _)) = self.state.run.handoff_pending.take() else {
            return;
        };
        cx.emit(AiPanelEvent::ResumeHandoffRequested { run_id, cancelled });
        cx.notify();
    }

    pub(super) fn install_acp_agent(&mut self, agent_id: String, cx: &mut Context<Self>) {
        let agent = self
            .state
            .config
            .settings
            .acp_agents
            .iter()
            .find(|a| a.id == agent_id)
            .cloned()
            .or_else(|| {
                transom_ai_contracts::default_acp_presets()
                    .into_iter()
                    .find(|a| a.id == agent_id)
            });
        let Some(agent) = agent else {
            return;
        };
        let install_command = if !agent.install_command.trim().is_empty() {
            agent.install_command
        } else {
            transom_ai_contracts::default_acp_presets()
                .into_iter()
                .find(|p| p.id == agent.id)
                .map(|p| p.install_command)
                .unwrap_or_default()
        };
        if install_command.trim().is_empty() {
            self.state.config.acp_install_status = Some((
                agent_id,
                false,
                rust_i18n::t!("ai.acp_install_no_cmd").to_string(),
            ));
            cx.notify();
            return;
        }
        self.send_ai_command(
            AiCommand::InstallAcpAgent {
                agent_id,
                name: agent.name,
                install_command,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn handle_handoff_focus_request(&self, command: String, cx: &mut Context<Self>) {
        cx.emit(AiPanelEvent::FocusTerminalForHandoff { command });
    }

    pub(super) fn toggle_card_id(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.state.conversation.expanded_cards.remove(&id) {
            self.state.conversation.expanded_cards.insert(id);
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
        self.tasks.copy_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1500))
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if this.copied_key.as_deref() == Some(key.as_str()) {
                    this.copied_key = None;
                    cx.notify();
                }
            }) {
                log::debug!("复制反馈重置时 AI 面板已关闭: {err}");
            }
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
        let v_rev = cx.entity();
        let v_jump = cx.entity();
        let v_sub = cx.entity();
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
            on_revert_remote_file: std::rc::Rc::new(
                move |host_label, target_path, backup_path, _, cx| {
                    v_rev.update(cx, |this, cx| {
                        let conversation_id =
                            this.state.conversation.active_conversation_id.clone();
                        this.send_ai_command(
                            transom_core::AiCommand::RevertRemoteFile {
                                host_label,
                                target_path,
                                backup_path,
                                conversation_id,
                            },
                            cx,
                        );
                    });
                },
            ),
            on_jump_to_terminal: std::rc::Rc::new(move |host_label, _, cx| {
                v_jump.update(cx, |_, cx| {
                    cx.emit(super::AiPanelEvent::JumpToTerminal { host_label });
                });
            }),
            on_subagent_action: std::rc::Rc::new(move |action, task_id, w, cx| {
                v_sub.update(cx, |this, cx| {
                    match action.as_str() {
                        "stop" => {
                            this.send_ai_command(
                                transom_core::AiCommand::CancelManagedTask { task_id },
                                cx,
                            );
                        }
                        "followup" => {
                            let msg = this.composer_input.read(cx).value().trim().to_owned();
                            if !msg.is_empty()
                                && this.send_ai_command(
                                    transom_core::AiCommand::SteerRun {
                                        run_id: task_id,
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
        let active_changed = self.state.conversation.active_session != session;
        let list_changed = self.state.conversation.connected_sessions != connected_sessions;
        if !active_changed && !list_changed {
            return;
        }
        self.state.conversation.connected_sessions = connected_sessions;
        let primary_tab = session.as_ref().map(|s| s.tab_id);
        self.state.conversation.extra_scope_tabs.retain(|tid| {
            Some(*tid) != primary_tab
                && self
                    .state
                    .conversation
                    .connected_sessions
                    .iter()
                    .any(|s| s.tab_id == *tid)
        });
        if !active_changed {
            cx.notify();
            return;
        }
        self.state.conversation.active_session = session;
        let scope = self.state.current_scope();
        let host_id = match scope {
            transom_ai_contracts::SessionScope::Host(id) => Some(id),
            transom_ai_contracts::SessionScope::Global => None,
        };
        self.state.conversation.memory_load_seq =
            self.state.conversation.memory_load_seq.wrapping_add(1);
        let load_seq = self.state.conversation.memory_load_seq;
        let store = self.ai_store.clone();
        self.tasks.memory_load = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = spawn_blocking_local(move || store.load_memory(host_id.as_deref())).await;
            let loaded = loaded.unwrap_or_else(|| {
                Err(transom_ai_contracts::AiError::StorageFailed {
                    target: "ai_memory_load".to_owned(),
                    reason: rust_i18n::t!("ai.background_task_failed").to_string(),
                })
            });
            if this
                .update_in(cx, |this, window, cx| {
                    if this.state.conversation.memory_load_seq != load_seq {
                        return;
                    }
                    match loaded {
                        Ok(memory) => {
                            this.state.memory = memory;
                            let global_text = this.state.memory.global_markdown.clone();
                            let host_text = this.state.memory.host_markdown.clone();
                            this.memory_form
                                .global_mem_input
                                .update(cx, |input, cx| input.set_value(global_text, window, cx));
                            this.memory_form
                                .host_mem_input
                                .update(cx, |input, cx| input.set_value(host_text, window, cx));
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
                log::debug!("AI 记忆读取完成时视图已关闭，结果被丢弃");
            }
        }));
        cx.notify();
    }

    /// 外壳注入用于 `@` 提及补全的可用目标列表（SSH 会话、容器引擎与本机）。
    pub(crate) fn set_mention_candidates(
        &mut self,
        candidates: Vec<super::state::AiMentionCandidate>,
        cx: &mut Context<Self>,
    ) {
        if self.state.composer.mention_candidates == candidates {
            return;
        }
        self.state.composer.mention_candidates = candidates;
        cx.notify();
    }

    pub(super) fn render_chat_composer(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let active_acp = self.active_acp_agent();
        let base_model_label = if let Some(agent) = &active_acp {
            if !agent.default_model.trim().is_empty() {
                agent.default_model.clone()
            } else {
                transom_ai_contracts::default_acp_presets()
                    .into_iter()
                    .find(|p| p.id == agent.id)
                    .map(|p| p.default_model)
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| agent.name.clone())
            }
        } else {
            self.state
                .config
                .settings
                .resolve_model(&self.state.config.selected_model_id)
                .map(|(_, m)| m.display_label().to_owned())
                .unwrap_or_else(|| rust_i18n::t!("ai.no_model").to_string())
        };
        let model_label = if active_acp.is_none() {
            match self.state.config.selected_effort {
                Some(eff) => format!("{base_model_label} · {}", eff.as_str()),
                None => base_model_label,
            }
        } else {
            base_model_label
        };
        let session_badge = self.state.conversation.active_session.as_ref().map(|s| {
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
        let active_acp_id = self.state.config.active_acp_agent_id.clone();
        let acp_missing_warning = active_acp.as_ref().and_then(|agent| {
            (!is_acp_command_available(&agent.command))
                .then(|| (agent.id.clone(), agent.name.clone(), agent.command.clone()))
        });
        let acp_label = active_acp
            .map(|a| a.name)
            .unwrap_or_else(|| rust_i18n::t!("ai.acp_builtin_mode").to_string());

        let primary_tab = self
            .state
            .conversation
            .active_session
            .as_ref()
            .map(|s| s.tab_id);
        let extra_host_options: Vec<ExtraHostOption> = if primary_tab.is_some() {
            self.state
                .conversation
                .connected_sessions
                .iter()
                .filter(|s| Some(s.tab_id) != primary_tab)
                .map(|s| ExtraHostOption {
                    tab_id: s.tab_id,
                    host_name: s.host_name.clone(),
                    selected: self.state.conversation.extra_scope_tabs.contains(&s.tab_id),
                })
                .collect()
        } else {
            Vec::new()
        };

        let view = cx.entity();
        let (v_stop, v_popup, v_extra, v_builtin, v_cfg, v_install, v_rm_att, v_paste, v_drop) = (
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

        let is_current_running = self.state.active_run_is_current();
        let running_tasks_count = if is_current_running {
            self.state
                .run
                .managed_tasks
                .iter()
                .filter(|t| t.dto.status == transom_core::AiManagedTaskStatus::Running)
                .count()
        } else {
            0
        };
        let v_pill_plan = view.clone();
        let v_pill_tasks = view.clone();
        let status_header = super::plan_card::render_status_pills_row(
            super::plan_card::StatusPillsParams {
                plan: is_current_running
                    .then_some(self.state.run.active_plan.as_ref())
                    .flatten(),
                focus: is_current_running
                    .then_some(self.state.run.active_focus.as_deref())
                    .flatten(),
                plan_expanded: self.state.view.status_panel_expanded,
                running_tasks_count,
                tasks_expanded: self.state.run.tasks_drawer_expanded,
                p,
            },
            move |_, cx| {
                v_pill_plan.update(cx, |this, cx| {
                    this.state.view.status_panel_expanded = !this.state.view.status_panel_expanded;
                    if this.state.view.status_panel_expanded {
                        this.state.run.tasks_drawer_expanded = false;
                    }
                    cx.notify();
                });
            },
            move |_, cx| {
                v_pill_tasks.update(cx, |this, cx| {
                    this.state.run.tasks_drawer_expanded = !this.state.run.tasks_drawer_expanded;
                    if this.state.run.tasks_drawer_expanded {
                        this.state.view.status_panel_expanded = false;
                    }
                    cx.notify();
                });
            },
        );

        render_composer(
            ComposerParams {
                input: &self.composer_input,
                is_running: is_current_running,
                model_label,
                execution_mode: self.state.config.execution_mode,
                session_badge,
                acp_label,
                active_acp_id,
                acp_missing_warning,
                acp_install_status: self.state.config.acp_install_status.as_ref(),
                active_popup: self.state.view.active_popup,
                toolbar_popover: self.render_toolbar_popover(p, cx),
                pending_attachments: &self.state.composer.pending_attachments,
                metrics: &self.state.conversation.metrics,
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
                        this.state.view.active_popup = AiActivePopup::None;
                        this.state.view.drawer = AiDrawerTab::Config;
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
