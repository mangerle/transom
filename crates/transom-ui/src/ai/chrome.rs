//! AI 面板顶栏、浮层与历史/配置抽屉装配辅助方法。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_ai::{AcpAgentConfig, default_acp_presets};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use super::AiPanelEvent;
use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::empty_state::{header_icon_btn, render_empty_state};
use super::metrics::{
    format_duration_zh, render_context_usage_popup, render_session_stats_popup,
    render_token_usage_popup,
};
use super::popups::{
    ModelPopupParams, render_backend_popup, render_mode_popup, render_model_popup,
};
use super::state::{AiActivePopup, AiDrawerTab};
use super::turn_nav::{collect_turn_nav_entries, render_turn_nav_rail};
use super::view::AiAssistantView;
use super::work_group::{
    DisplayBlock, WorkGroupParams, chat_item_id, group_chat_items, render_work_group,
};
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};

impl AiAssistantView {
    pub(super) fn render_header(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let v_new = cx.entity();
        let v_hist = cx.entity();
        let v_mem = cx.entity();
        let v_skills = cx.entity();
        let v_cfg = cx.entity();
        let v_close = cx.entity();

        let title = self
            .state
            .active_conversation_title
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| rust_i18n::t!("ai.title").to_string());

        let engine_icon = super::popups::backend_icon(self.state.active_acp_agent_id.as_deref());
        h_flex()
            .w_full()
            .h(rems(size::TAB_BAR_HEIGHT))
            .px(rems(space::SM))
            .justify_between()
            .items_center()
            .gap(rems(space::XS))
            .border_b_1()
            .border_color(p.divider(p.border))
            .bg(p.background)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.0))
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(p.primary)
                            .child(AppIconElement::new(engine_icon).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rems(typography::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(title),
                    ),
            )
            .child(
                h_flex()
                    .gap(rems(space::XXS))
                    .flex_shrink_0()
                    .child(header_icon_btn(
                        "ai-new-chat",
                        AppIcon::Ui(UiIcon::ADD),
                        rust_i18n::t!("ai.new_chat").to_string(),
                        false,
                        p,
                        move |_, cx| v_new.update(cx, |this, cx| this.clear_conversation(cx)),
                    ))
                    .child(header_icon_btn(
                        "ai-hist-btn",
                        AppIcon::Ui(UiIcon::HISTORY),
                        rust_i18n::t!("ai.history_title").to_string(),
                        self.state.drawer == AiDrawerTab::History,
                        p,
                        move |_, cx| {
                            v_hist.update(cx, |this, cx| {
                                this.state.active_popup = AiActivePopup::None;
                                this.state.drawer = if this.state.drawer == AiDrawerTab::History {
                                    AiDrawerTab::Chat
                                } else {
                                    AiDrawerTab::History
                                };
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-mem-btn",
                        AppIcon::Custom(CustomIcon::MEMORY),
                        rust_i18n::t!("ai.memory").to_string(),
                        self.state.drawer == AiDrawerTab::Memory,
                        p,
                        move |_, cx| {
                            v_mem.update(cx, |this, cx| {
                                this.state.active_popup = AiActivePopup::None;
                                this.state.drawer = if this.state.drawer == AiDrawerTab::Memory {
                                    AiDrawerTab::Chat
                                } else {
                                    AiDrawerTab::Memory
                                };
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-skills-btn",
                        AppIcon::Ui(UiIcon::RUN_ANYTHING),
                        rust_i18n::t!("ai.skills_title").to_string(),
                        self.state.drawer == AiDrawerTab::Skills,
                        p,
                        move |_, cx| {
                            v_skills.update(cx, |this, cx| {
                                this.state.active_popup = AiActivePopup::None;
                                if this.state.drawer == AiDrawerTab::Skills {
                                    this.state.drawer = AiDrawerTab::Chat;
                                } else {
                                    this.refresh_skills_list();
                                    this.state.drawer = AiDrawerTab::Skills;
                                }
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-cfg-btn",
                        AppIcon::Ui(UiIcon::SETTINGS),
                        rust_i18n::t!("ai.model").to_string(),
                        self.state.drawer == AiDrawerTab::Config,
                        p,
                        move |_, cx| {
                            v_cfg.update(cx, |this, cx| {
                                this.state.active_popup = AiActivePopup::None;
                                this.state.drawer = if this.state.drawer == AiDrawerTab::Config {
                                    AiDrawerTab::Chat
                                } else {
                                    AiDrawerTab::Config
                                };
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-close-btn",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("ai.collapse").to_string(),
                        false,
                        p,
                        move |_, cx| {
                            v_close.update(cx, |_, cx| cx.emit(AiPanelEvent::CloseRequested));
                        },
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_toolbar_popover(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<(AiActivePopup, AnyElement)> {
        if !self.current_slash_items(cx).is_empty() {
            return None;
        }
        let view = cx.entity();
        let kind = self.state.active_popup;
        let pop = match kind {
            AiActivePopup::None => return None,
            AiActivePopup::Backend => {
                let v_cfg = view.clone();
                Some(render_backend_popup(
                    &self.state.settings,
                    self.state.active_acp_agent_id.as_deref(),
                    p,
                    move |id, _, cx| {
                        view.update(cx, |this, cx| this.select_acp_agent(id, cx));
                    },
                    move |_, cx| {
                        v_cfg.update(cx, |this, cx| {
                            this.state.active_popup = AiActivePopup::None;
                            this.state.drawer = AiDrawerTab::Config;
                            cx.notify();
                        });
                    },
                ))
            }
            AiActivePopup::Mode => Some(render_mode_popup(
                self.state.execution_mode,
                p,
                move |mode, _, cx| {
                    view.update(cx, |this, cx| this.select_execution_mode(mode, cx));
                },
            )),
            AiActivePopup::Model => {
                let v_eff = view.clone();
                let v_cfg = view.clone();
                let active_acp = self.active_acp_agent();
                Some(render_model_popup(
                    ModelPopupParams {
                        settings: &self.state.settings,
                        active_acp: active_acp.as_ref(),
                        selected_model_id: &self.state.selected_model_id,
                        selected_effort: self.state.selected_effort,
                        p,
                    },
                    move |id, _, cx| {
                        view.update(cx, |this, cx| this.select_model(id, cx));
                    },
                    move |eff, _, cx| {
                        v_eff.update(cx, |this, cx| this.select_effort(eff, cx));
                    },
                    move |_, cx| {
                        v_cfg.update(cx, |this, cx| {
                            this.state.active_popup = AiActivePopup::None;
                            this.state.drawer = AiDrawerTab::Config;
                            cx.notify();
                        });
                    },
                ))
            }
            AiActivePopup::SessionStats => Some(render_session_stats_popup(&self.state.metrics, p)),
            AiActivePopup::TokenUsage => Some(render_token_usage_popup(&self.state.metrics, p)),
            AiActivePopup::ContextUsage => {
                let v_cmp = view;
                Some(render_context_usage_popup(
                    &self.state.metrics,
                    self.active_context_window(),
                    self.state.items.len(),
                    p,
                    move |_, cx| {
                        v_cmp.update(cx, |this, cx| this.compact_conversation(String::new(), cx));
                    },
                ))
            }
        }?;
        Some((kind, pop))
    }

    pub(super) fn active_acp_agent(&self) -> Option<AcpAgentConfig> {
        let id = self.state.active_acp_agent_id.as_deref()?;
        self.state
            .settings
            .acp_agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .or_else(|| default_acp_presets().into_iter().find(|a| a.id == id))
    }

    pub(super) fn render_message_stream(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut msg_list = v_flex()
            .id("ai-msg-stream")
            .flex_1()
            .w_full()
            .min_w_0()
            .h_full()
            .px(rems(space::MD))
            .py(rems(space::SM))
            .gap(rems(0.75))
            .overflow_y_scroll()
            .track_scroll(&self.msg_scroll_handle);

        let mut nav_entries = Vec::new();
        if self.state.items.is_empty() {
            let active = self.active_acp_agent();
            let v_quick = cx.entity();
            msg_list = msg_list.child(render_empty_state(
                active.as_ref().map(|a| a.id.as_str()),
                active.as_ref().map(|a| a.name.as_str()),
                p,
                move |prompt, _, cx| {
                    v_quick.update(cx, |this, cx| this.send_prompt_text(prompt, cx));
                },
            ));
        } else {
            let is_running = self.state.running_run_id.is_some();
            let live_elapsed = self.state.run_started_at.map(|t| t.elapsed().as_secs());
            let blocks = group_chat_items(&self.state.items, is_running);
            nav_entries = collect_turn_nav_entries(&blocks);
            let has_active_wg = blocks.last().is_some_and(|b| {
                matches!(
                    b,
                    DisplayBlock::WorkGroup {
                        in_progress: true,
                        ..
                    }
                )
            });
            let code_callbacks = self.build_code_callbacks(cx);
            let copied_key = self.copied_key.as_deref();
            let has_active_terminal = self.state.active_session.is_some();
            for block in blocks {
                let v_exp = cx.entity();
                match block {
                    DisplayBlock::Single(item) => {
                        let cid = chat_item_id(item).to_owned();
                        let expanded = self.state.expanded_cards.contains(&cid);
                        let v_rb = cx.entity();
                        msg_list = msg_list.child(render_chat_item(
                            ChatItemRenderParams {
                                item,
                                expanded,
                                copied_key,
                                has_active_terminal,
                                p,
                            },
                            ChatItemCallbacks {
                                on_toggle_expand: std::rc::Rc::new(move |_, cx| {
                                    v_exp.update(cx, |this, cx| {
                                        this.toggle_card_id(cid.clone(), cx);
                                    });
                                }),
                                on_rollback: std::rc::Rc::new(move |entry_id, _, cx| {
                                    v_rb.update(cx, |this, cx| {
                                        this.rollback_checkpoint(entry_id, cx);
                                    });
                                }),
                                code: code_callbacks.clone(),
                            },
                        ));
                    }
                    DisplayBlock::WorkGroup {
                        id,
                        items,
                        tool_steps,
                        ok_steps,
                        duration_ms,
                        in_progress,
                    } => {
                        msg_list = msg_list.child(render_work_group(
                            WorkGroupParams {
                                id: &id,
                                items: &items,
                                tool_steps,
                                ok_steps,
                                duration_ms,
                                in_progress,
                                expanded_cards: &self.state.expanded_cards,
                                active_focus: self.state.active_focus.as_deref(),
                                live_elapsed_secs: live_elapsed,
                                copied_key,
                                has_active_terminal,
                                p,
                            },
                            move |tid, _, cx| {
                                v_exp.update(cx, |this, cx| this.toggle_card_id(tid, cx));
                            },
                            code_callbacks.clone(),
                        ));
                    }
                }
            }
            if is_running && !has_active_wg {
                let focus = self
                    .state
                    .active_focus
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| rust_i18n::t!("ai.working_default").to_string());
                let elapsed_str = live_elapsed
                    .filter(|&s| s > 0)
                    .map(|s| format_duration_zh(s.saturating_mul(1000)))
                    .unwrap_or_default();
                let label = if elapsed_str.is_empty() {
                    rust_i18n::t!("ai.working_running", focus = focus).to_string()
                } else {
                    format!(
                        "{}  {elapsed_str}",
                        rust_i18n::t!("ai.working_running", focus = focus)
                    )
                };
                msg_list = msg_list.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .px(rems(space::SM))
                        .py(rems(space::XS))
                        .gap(rems(space::XS))
                        .items_center()
                        .rounded(p.radius.sm)
                        .bg(p.secondary)
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(p.primary)
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .child(label),
                        ),
                );
            }
        }

        if let Some(err) = &self.state.status_error {
            msg_list = msg_list.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(px(6.0))
                    .items_start()
                    .rounded(p.radius.md)
                    .bg(p.secondary)
                    .border_1()
                    .border_color(p.danger)
                    .child(
                        div()
                            .flex_shrink_0()
                            .mt(px(2.0))
                            .text_color(p.danger)
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::ERROR)).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.danger)
                            .child(err.clone()),
                    ),
            );
        }
        let max_y = self.msg_scroll_handle.max_offset().y;
        let off_y = -self.msg_scroll_handle.offset().y;
        let show_scroll_bottom = max_y > px(40.0) && (max_y - off_y) > px(40.0);

        let mut body_row = h_flex().flex_1().w_full().min_w_0().overflow_hidden();
        if !nav_entries.is_empty() {
            let view = cx.entity();
            let rail = render_turn_nav_rail(
                &nav_entries,
                self.active_nav_dot,
                p,
                move |dot_idx, block_idx, _, cx| {
                    view.update(cx, |this, cx| {
                        this.msg_scroll_handle.scroll_to_item(block_idx);
                        this.active_nav_dot = Some(dot_idx);
                        cx.notify();
                    });
                },
            );
            body_row = body_row.child(rail);
        }
        body_row = body_row.child(msg_list);

        let mut wrapper = div()
            .relative()
            .flex_1()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .child(body_row);
        if show_scroll_bottom {
            let v_btm = cx.entity();
            wrapper = wrapper.child(super::plan_card::render_scroll_bottom_button(
                p,
                move |_, cx| {
                    v_btm.update(cx, |this, cx| {
                        let target = this.msg_scroll_handle.max_offset().y;
                        this.msg_scroll_handle.set_offset(point(px(0.0), -target));
                        cx.notify();
                    });
                },
            ));
        }
        wrapper.into_any_element()
    }
}
