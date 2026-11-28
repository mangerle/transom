//! AI 面板浮层、消息流与任务栏装配辅助方法。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ai_contracts::{AcpAgentConfig, default_acp_presets};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::empty_state::render_empty_state;
use super::metrics::{
    format_duration_zh, render_context_usage_popup, render_session_stats_popup,
    render_token_usage_popup,
};
use super::popups::{
    ModelPopupParams, render_backend_popup, render_mode_popup, render_model_popup,
};
use super::state::{AiActivePopup, AiDrawerTab};
use super::turn_nav::{collect_turn_nav_entries, render_turn_nav_rail, resolve_active_nav_dot};
use super::view::AiAssistantView;
use super::work_group::{
    DisplayBlock, WorkGroupParams, chat_item_id, group_chat_items, render_work_group,
};
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

impl AiAssistantView {
    pub(super) fn render_toolbar_popover(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<(AiActivePopup, AnyElement)> {
        if !self.current_slash_items(cx).is_empty() {
            return None;
        }
        let view = cx.entity();
        let kind = self.state.view.active_popup;
        let pop = match kind {
            AiActivePopup::None => return None,
            AiActivePopup::Backend => {
                let v_cfg = view.clone();
                Some(render_backend_popup(
                    &self.state.config.settings,
                    self.state.config.active_acp_agent_id.as_deref(),
                    p,
                    move |id, _, cx| {
                        view.update(cx, |this, cx| this.select_acp_agent(id, cx));
                    },
                    move |_, cx| {
                        v_cfg.update(cx, |this, cx| {
                            this.state.view.active_popup = AiActivePopup::None;
                            this.state.view.drawer = AiDrawerTab::Config;
                            cx.notify();
                        });
                    },
                ))
            }
            AiActivePopup::Mode => Some(render_mode_popup(
                self.state.config.execution_mode,
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
                        settings: &self.state.config.settings,
                        active_acp: active_acp.as_ref(),
                        selected_model_id: &self.state.config.selected_model_id,
                        selected_effort: self.state.config.selected_effort,
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
                            this.state.view.active_popup = AiActivePopup::None;
                            this.state.view.drawer = AiDrawerTab::Config;
                            cx.notify();
                        });
                    },
                ))
            }
            AiActivePopup::SessionStats => Some(render_session_stats_popup(
                &self.state.conversation.metrics,
                p,
            )),
            AiActivePopup::TokenUsage => Some(render_token_usage_popup(
                &self.state.conversation.metrics,
                p,
            )),
            AiActivePopup::ContextUsage => {
                let v_cmp = view;
                Some(render_context_usage_popup(
                    &self.state.conversation.metrics,
                    self.active_context_window(),
                    self.state.conversation.items.len(),
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
        let id = self.state.config.active_acp_agent_id.as_deref()?;
        self.state
            .config
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
            .min_h_0()
            .h_full()
            .px(rems(space::LG))
            .pt(rems(space::MD))
            .pb(rems(space::LG))
            .gap(rems(0.75))
            .overflow_y_scroll()
            .track_scroll(&self.msg_scroll_handle)
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                let dy = event.delta.pixel_delta(px(16.0)).y;
                if dy > px(0.5) {
                    // 用户主动向上滚动：锁定自动滚底，避免抢夺视口。
                    this.auto_scroll_locked = true;
                    this.active_nav_dot = None;
                    cx.notify();
                } else if dy < px(-0.5) {
                    // 用户向下滚动时若已接近底部，则自动恢复跟踪滚动。
                    let max_y = this.msg_scroll_handle.max_offset().y;
                    let off_y = -this.msg_scroll_handle.offset().y;
                    if max_y - off_y <= px(24.0) {
                        this.auto_scroll_locked = false;
                    }
                    this.active_nav_dot = None;
                    cx.notify();
                }
            }));

        let mut nav_entries = Vec::new();
        if self.state.conversation.items.is_empty() {
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
            let is_running = self.state.active_run_is_current();
            let live_elapsed = self
                .state
                .current_run_started_at()
                .map(|time| time.elapsed().as_secs());
            let items = &self.state.conversation.items;
            let fallback_blocks;
            let blocks = if let Some(cache) = self.message_block_cache.as_ref().filter(|cache| {
                cache.item_count == items.len()
                    && cache.is_running == is_running
                    && cache.group_revision == self.state.conversation.group_revision
            }) {
                cache.blocks.as_slice()
            } else {
                fallback_blocks = group_chat_items(items, is_running);
                fallback_blocks.as_slice()
            };
            let render_start = blocks
                .len()
                .saturating_sub(self.state.conversation.render_window_blocks);
            let visible_blocks = &blocks[render_start..];
            nav_entries = collect_turn_nav_entries(items, visible_blocks);
            if render_start > 0 {
                for entry in &mut nav_entries {
                    entry.block_idx = entry.block_idx.saturating_add(1);
                }
            }
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
            let has_active_terminal = self.state.conversation.active_session.is_some();
            if render_start > 0 {
                let v_older = cx.entity();
                msg_list = msg_list.child(
                    Button::new("ai-load-older-messages")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("ai.load_earlier_messages").to_string())
                        .on_click(move |_, _, cx| {
                            v_older.update(cx, |this, cx| {
                                this.state.conversation.render_window_blocks = this
                                    .state
                                    .conversation
                                    .render_window_blocks
                                    .saturating_add(160);
                                cx.notify();
                            });
                        }),
                );
            }
            for (b_idx, block) in blocks.iter().enumerate().skip(render_start) {
                let v_exp = cx.entity();
                match block {
                    DisplayBlock::Single(item_index) => {
                        let item = &items[*item_index];
                        let cid = chat_item_id(item).to_owned();
                        let expanded = self.state.conversation.expanded_cards.contains(&cid);
                        let full_key = format!("full-out-{cid}");
                        let full_output_expanded =
                            self.state.conversation.expanded_cards.contains(&full_key);
                        let is_follow_user =
                            b_idx > 0 && matches!(item, super::state::UiChatItem::UserText { .. });
                        let after_work_group = b_idx > 0
                            && matches!(blocks[b_idx - 1], DisplayBlock::WorkGroup { .. })
                            && matches!(item, super::state::UiChatItem::AssistantText { .. });
                        let v_rb = cx.entity();
                        let v_full = cx.entity();
                        let subagent_task = super::work_group::find_subagent_task(
                            &self.state.run.managed_tasks,
                            item,
                        );
                        let item_el = render_chat_item(
                            ChatItemRenderParams {
                                item,
                                expanded,
                                full_output_expanded,
                                copied_key,
                                has_active_terminal,
                                subagent_task,
                                p,
                            },
                            ChatItemCallbacks {
                                on_toggle_expand: std::rc::Rc::new(move |_, cx| {
                                    v_exp.update(cx, |this, cx| {
                                        this.toggle_card_id(cid.clone(), cx);
                                    });
                                }),
                                on_toggle_full_output: std::rc::Rc::new(move |_, cx| {
                                    v_full.update(cx, |this, cx| {
                                        this.toggle_card_id(full_key.clone(), cx);
                                    });
                                }),
                                on_rollback: std::rc::Rc::new(move |entry_id, _, cx| {
                                    v_rb.update(cx, |this, cx| {
                                        this.rollback_checkpoint(entry_id, cx);
                                    });
                                }),
                                code: code_callbacks.clone(),
                            },
                        );
                        if is_follow_user {
                            msg_list =
                                msg_list.child(div().w_full().pt(rems(space::SM)).child(item_el));
                        } else if after_work_group {
                            msg_list = msg_list.child(div().w_full().pt(px(4.0)).child(item_el));
                        } else {
                            msg_list = msg_list.child(item_el);
                        }
                    }
                    DisplayBlock::WorkGroup {
                        id,
                        start,
                        end,
                        in_progress,
                    } => {
                        msg_list = msg_list.child(render_work_group(
                            WorkGroupParams {
                                id,
                                items: &items[*start..*end],
                                in_progress: *in_progress,
                                expanded_cards: &self.state.conversation.expanded_cards,
                                active_focus: self.state.run.active_focus.as_deref(),
                                live_elapsed_secs: live_elapsed,
                                copied_key,
                                has_active_terminal,
                                managed_tasks: &self.state.run.managed_tasks,
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
                    .run
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
                let label = super::work_group::format_running_focus(&focus);
                let mut row =
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .py(px(4.0))
                        .gap(rems(space::XS))
                        .justify_between()
                        .items_center()
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(6.0))
                                .items_center()
                                .overflow_hidden()
                                .child(div().flex_shrink_0().text_color(p.primary).child(
                                    AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small(),
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(rems(typography::CAPTION))
                                        .text_color(p.text.muted)
                                        .child(label),
                                ),
                        );
                if !elapsed_str.is_empty() {
                    row = row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(elapsed_str),
                    );
                }
                msg_list = msg_list.child(row);
            }
        }

        if let Some(err) = &self.state.view.status_error {
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
        let auto_following = self.state.active_run_is_current() && !self.auto_scroll_locked;
        if auto_following {
            // 流式输出且用户未主动向上滚动时，每帧维持贴底，避免高度变化导致半路停住或按钮闪烁。
            self.msg_scroll_handle.scroll_to_bottom();
        }
        let max_y = self.msg_scroll_handle.max_offset().y;
        let off_y = -self.msg_scroll_handle.offset().y;
        let show_scroll_bottom = !auto_following && max_y > px(40.0) && (max_y - off_y) > px(40.0);

        let mut body_row = h_flex()
            .flex_1()
            .w_full()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        if !nav_entries.is_empty() {
            let view = cx.entity();
            let top_item = self.msg_scroll_handle.top_item();
            let computed_dot = resolve_active_nav_dot(&nav_entries, top_item, !show_scroll_bottom);
            let active_dot = self.active_nav_dot.or(computed_dot);
            let rail = render_turn_nav_rail(
                &nav_entries,
                active_dot,
                p,
                move |dot_idx, block_idx, _, cx| {
                    view.update(cx, |this, cx| {
                        this.auto_scroll_locked = true;
                        this.msg_scroll_handle.scroll_to_item(block_idx);
                        this.active_nav_dot = Some(dot_idx);
                        cx.notify();
                    });
                },
            );
            body_row = body_row.child(rail);
        }
        body_row = body_row.child(msg_list);

        let mut wrapper = v_flex()
            .relative()
            .flex_1()
            .w_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(body_row);
        if show_scroll_bottom {
            let v_btm = cx.entity();
            wrapper = wrapper.child(super::plan_card::render_scroll_bottom_button(
                p,
                move |_, cx| {
                    v_btm.update(cx, |this, cx| {
                        this.auto_scroll_locked = false;
                        this.active_nav_dot = None;
                        this.msg_scroll_handle.scroll_to_bottom();
                        cx.notify();
                    });
                },
            ));
        }
        wrapper.into_any_element()
    }
}
