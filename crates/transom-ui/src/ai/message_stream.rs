//! 独立的 AI 消息流 GPUI 实体：承载消息列表渲染、流式局部增量更新、滚动跟踪与导航点。
//!
//! 设计原理（Why）：
//! - 将高频流式 `TextDelta` / `ReasoningDelta` 重绘隔离在 [`MessageStreamView`] 内部，
//!   流式输出期间仅调用本实体的 `cx.notify()`，避免顶栏、抽屉、任务栏与输入区陪跑重绘；
//! - 严格遵循单向数据流：父视图 [`super::view::AiAssistantView`] 通过 `sync_snapshot` /
//!   `apply_streaming_delta` 向下推送投影数据，用户交互通过 [`MessageStreamEvent`] 向上冒泡，
//!   渲染链路严禁任何 `read_entity` 反向读取。

use std::rc::Rc;
use std::time::Instant;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_core::AiAppEvent;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::empty_state::render_empty_state;
use super::markdown_blocks::CodeBlockCallbacks;
use super::metrics::format_duration_zh;
use super::plan_card::render_scroll_bottom_button;
use super::state::{MessageStreamModel, UiChatItem};
use super::task_bar::UiManagedTask;
use super::turn_nav::{collect_turn_nav_entries, render_turn_nav_rail, resolve_active_nav_dot};
use super::work_group::{
    DisplayBlock, MessageBlockCache, WorkGroupParams, chat_item_id, find_subagent_task,
    format_running_focus, group_chat_items, render_work_group,
};
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 消息流实体向父视图发出的交互意图事件。
#[derive(Debug, Clone)]
pub(in crate::app::ai) enum MessageStreamEvent {
    ToggleCard(String),
    RollbackCheckpoint(String),
    SendQuickPrompt(String),
    CopyText {
        key: String,
        text: String,
    },
    SendToTerminal(String),
    SaveSnippet {
        name: String,
        command: String,
    },
    RevertRemoteFile {
        host: String,
        target: String,
        backup: String,
    },
    JumpToTerminal(String),
    SubagentAction {
        action: String,
        task_id: String,
    },
    LoadOlderMessages,
    PreviewImage {
        title: String,
        base64: String,
    },
}

/// 由父视图单向注入的消息流运行态元信息快照（不含消息列表深拷贝）。
pub(in crate::app::ai) struct MessageStreamSnapshot {
    pub(in crate::app::ai) is_running: bool,
    pub(in crate::app::ai) run_started_at: Option<Instant>,
    pub(in crate::app::ai) active_focus: Option<String>,
    pub(in crate::app::ai) status_error: Option<String>,
    pub(in crate::app::ai) copied_key: Option<String>,
    pub(in crate::app::ai) has_active_terminal: bool,
    pub(in crate::app::ai) active_acp: Option<(String, String)>,
    pub(in crate::app::ai) managed_tasks: Vec<UiManagedTask>,
}

/// 独立的 AI 消息流视图实体（消息流单一数据源的唯一持有者）。
pub(in crate::app::ai) struct MessageStreamView {
    model: MessageStreamModel,
    is_running: bool,
    run_started_at: Option<Instant>,
    active_focus: Option<String>,
    status_error: Option<String>,
    copied_key: Option<String>,
    has_active_terminal: bool,
    active_acp: Option<(String, String)>,
    managed_tasks: Vec<UiManagedTask>,
    block_cache: Option<MessageBlockCache>,
    code_callbacks: Option<CodeBlockCallbacks>,
    scroll_handle: ScrollHandle,
    auto_scroll_locked: bool,
    active_nav_dot: Option<usize>,
}

impl EventEmitter<MessageStreamEvent> for MessageStreamView {}

impl MessageStreamView {
    pub(in crate::app::ai) fn new() -> Self {
        Self {
            model: MessageStreamModel::default(),
            is_running: false,
            run_started_at: None,
            active_focus: None,
            status_error: None,
            copied_key: None,
            has_active_terminal: false,
            active_acp: None,
            managed_tasks: Vec::new(),
            block_cache: None,
            code_callbacks: None,
            scroll_handle: ScrollHandle::new(),
            auto_scroll_locked: false,
            active_nav_dot: None,
        }
    }

    /// 返回当前消息条目总数。
    pub(in crate::app::ai) fn item_count(&self) -> usize {
        self.model.item_count()
    }

    /// 返回最后一条已完成助手回复的 `(id, text)`。
    pub(in crate::app::ai) fn last_completed_assistant(&self) -> Option<(&str, &str)> {
        self.model.last_completed_assistant()
    }

    /// 返回最后一条助手消息的 ID。
    pub(in crate::app::ai) fn last_assistant_id(&self) -> Option<&str> {
        self.model.last_assistant_id()
    }

    /// 同步父视图运行态元信息快照（零消息列表克隆）。
    pub(in crate::app::ai) fn sync_snapshot(
        &mut self,
        snap: MessageStreamSnapshot,
        reset_scroll_lock: bool,
        scroll_to_bottom: bool,
        cx: &mut Context<Self>,
    ) {
        self.is_running = snap.is_running;
        self.run_started_at = snap.run_started_at;
        self.active_focus = snap.active_focus;
        self.status_error = snap.status_error;
        self.copied_key = snap.copied_key;
        self.has_active_terminal = snap.has_active_terminal;
        self.active_acp = snap.active_acp;
        self.managed_tasks = snap.managed_tasks;
        if reset_scroll_lock {
            self.auto_scroll_locked = false;
            self.active_nav_dot = None;
        }
        self.refresh_block_cache();
        if scroll_to_bottom && !self.auto_scroll_locked {
            self.scroll_handle.scroll_to_bottom();
        }
        cx.notify();
    }

    /// 原地消费控制平面事件并更新唯一消息流数据源。
    pub(in crate::app::ai) fn apply_app_event(
        &mut self,
        event: &AiAppEvent,
        active_model_name: Option<String>,
        is_current_conversation: bool,
        cx: &mut Context<Self>,
    ) {
        self.model
            .apply_app_event(event, active_model_name, is_current_conversation);
        self.refresh_block_cache();
        cx.notify();
    }

    /// 整表替换历史消息条目（会话切换 / 回滚加载完成时调用）。
    pub(in crate::app::ai) fn replace_items(
        &mut self,
        items: Vec<UiChatItem>,
        cx: &mut Context<Self>,
    ) {
        self.model.replace_items(items);
        self.refresh_block_cache();
        cx.notify();
    }

    /// 清空全部消息流。
    pub(in crate::app::ai) fn clear_items(&mut self, cx: &mut Context<Self>) {
        self.model.clear();
        self.refresh_block_cache();
        cx.notify();
    }

    /// 追加单条消息条目。
    pub(in crate::app::ai) fn push_item(&mut self, item: UiChatItem, cx: &mut Context<Self>) {
        self.model.push_item(item);
        self.refresh_block_cache();
        cx.notify();
    }

    /// 按消息 ID 删除条目。
    pub(in crate::app::ai) fn remove_item_by_id(
        &mut self,
        target_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.model.remove_item_by_id(target_id);
        self.refresh_block_cache();
        cx.notify();
    }

    /// 切换指定卡片或输出块的展开/折叠状态。
    pub(in crate::app::ai) fn toggle_expanded(&mut self, id: String, cx: &mut Context<Self>) {
        self.model.toggle_expanded(id);
        cx.notify();
    }

    /// 向上扩充历史消息可见块窗口。
    pub(in crate::app::ai) fn expand_older_window(&mut self, step: usize, cx: &mut Context<Self>) {
        self.model.expand_older_window(step);
        cx.notify();
    }

    /// 仅原地追加流式文本/思考增量并局部重绘本实体（零整树重绘、零全量列表拷贝）。
    pub(in crate::app::ai) fn apply_streaming_delta(
        &mut self,
        is_reasoning: bool,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        self.model.apply_streaming_delta(is_reasoning, delta);
        self.refresh_block_cache();
        if !self.auto_scroll_locked {
            self.scroll_handle.scroll_to_bottom();
        }
        cx.notify();
    }

    /// 显式解锁自动滚底并滚动到消息流底部（例如发送新提问时）。
    pub(in crate::app::ai) fn reset_and_scroll_bottom(&mut self, cx: &mut Context<Self>) {
        self.auto_scroll_locked = false;
        self.active_nav_dot = None;
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    fn refresh_block_cache(&mut self) {
        let is_running = self.is_running;
        let item_count = self.model.item_count();
        let group_revision = self.model.group_revision();
        let cache_is_valid = self.block_cache.as_ref().is_some_and(|cache| {
            cache.item_count == item_count
                && cache.is_running == is_running
                && cache.group_revision == group_revision
        });
        if cache_is_valid {
            return;
        }
        let blocks = group_chat_items(self.model.items(), is_running);
        self.block_cache = Some(MessageBlockCache {
            item_count,
            is_running,
            group_revision,
            blocks,
        });
    }

    fn cached_code_callbacks(&mut self, cx: &mut Context<Self>) -> CodeBlockCallbacks {
        if let Some(cb) = &self.code_callbacks {
            return cb.clone();
        }
        let cb = self.build_code_callbacks(cx);
        self.code_callbacks = Some(cb.clone());
        cb
    }

    fn build_code_callbacks(&self, cx: &mut Context<Self>) -> CodeBlockCallbacks {
        let v_copy = cx.entity();
        let v_send = cx.entity();
        let v_snip = cx.entity();
        let v_rev = cx.entity();
        let v_jump = cx.entity();
        let v_sub = cx.entity();
        CodeBlockCallbacks {
            on_copy: Rc::new(move |key, text, _, cx| {
                v_copy.update(cx, |_, cx| {
                    cx.emit(MessageStreamEvent::CopyText { key, text });
                });
            }),
            on_send_to_terminal: Rc::new(move |command, _, cx| {
                v_send.update(cx, |_, cx| {
                    cx.emit(MessageStreamEvent::SendToTerminal(command));
                });
            }),
            on_save_snippet: Rc::new(move |name, command, _, cx| {
                v_snip.update(cx, |_, cx| {
                    cx.emit(MessageStreamEvent::SaveSnippet { name, command });
                });
            }),
            on_revert_remote_file: Rc::new(move |host, target, backup, _, cx| {
                v_rev.update(cx, |_, cx| {
                    cx.emit(MessageStreamEvent::RevertRemoteFile {
                        host,
                        target,
                        backup,
                    });
                });
            }),
            on_jump_to_terminal: Rc::new(move |host, _, cx| {
                v_jump.update(cx, |_, cx| {
                    cx.emit(MessageStreamEvent::JumpToTerminal(host));
                });
            }),
            on_subagent_action: Rc::new(move |action, task_id, _, cx| {
                v_sub.update(cx, |_, cx| {
                    cx.emit(MessageStreamEvent::SubagentAction { action, task_id });
                });
            }),
        }
    }
}

impl Render for MessageStreamView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let p = &p;
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
            .track_scroll(&self.scroll_handle)
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                let dy = event.delta.pixel_delta(px(16.0)).y;
                if dy > px(0.5) {
                    this.auto_scroll_locked = true;
                    this.active_nav_dot = None;
                    cx.notify();
                } else if dy < px(-0.5) {
                    let max_y = this.scroll_handle.max_offset().y;
                    let off_y = -this.scroll_handle.offset().y;
                    if max_y - off_y <= px(24.0) {
                        this.auto_scroll_locked = false;
                    }
                    this.active_nav_dot = None;
                    cx.notify();
                }
            }));

        let mut nav_entries = Vec::new();
        if self.model.is_empty() {
            let v_quick = cx.entity();
            msg_list = msg_list.child(render_empty_state(
                self.active_acp.as_ref().map(|(id, _)| id.as_str()),
                self.active_acp.as_ref().map(|(_, name)| name.as_str()),
                self.has_active_terminal,
                p,
                move |prompt, _, cx| {
                    v_quick.update(cx, |_, cx| {
                        cx.emit(MessageStreamEvent::SendQuickPrompt(prompt));
                    });
                },
            ));
        } else {
            let code_callbacks = self.cached_code_callbacks(cx);
            let is_running = self.is_running;
            let live_elapsed = self.run_started_at.map(|t| t.elapsed().as_secs());
            let items = self.model.items();
            let expanded_cards = self.model.expanded_cards();
            let group_revision = self.model.group_revision();
            let render_window_blocks = self.model.render_window_blocks();
            let fallback_blocks;
            let blocks = if let Some(cache) = self.block_cache.as_ref().filter(|cache| {
                cache.item_count == items.len()
                    && cache.is_running == is_running
                    && cache.group_revision == group_revision
            }) {
                cache.blocks.as_slice()
            } else {
                fallback_blocks = group_chat_items(items, is_running);
                fallback_blocks.as_slice()
            };
            let render_start = blocks.len().saturating_sub(render_window_blocks);
            let visible_blocks = &blocks[render_start..];
            nav_entries = collect_turn_nav_entries(items, visible_blocks);
            if render_start > 0 {
                for entry in &mut nav_entries {
                    entry.block_idx = entry.block_idx.saturating_add(1);
                }
                let v_older = cx.entity();
                msg_list = msg_list.child(
                    Button::new("ai-load-older-messages")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("ai.load_earlier_messages").to_string())
                        .on_click(move |_, _, cx| {
                            v_older.update(cx, |_, cx| {
                                cx.emit(MessageStreamEvent::LoadOlderMessages);
                            });
                        }),
                );
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
            let copied_key = self.copied_key.as_deref();
            let has_active_terminal = self.has_active_terminal;
            let frame_now = chrono::Utc::now();

            for (b_idx, block) in blocks.iter().enumerate().skip(render_start) {
                let v_exp = cx.entity();
                match block {
                    DisplayBlock::Single(item_index) => {
                        let item = &items[*item_index];
                        let cid = chat_item_id(item).to_owned();
                        let expanded = expanded_cards.contains(&cid);
                        let full_key = format!("full-out-{cid}");
                        let full_output_expanded = expanded_cards.contains(&full_key);
                        let is_follow_user =
                            b_idx > 0 && matches!(item, UiChatItem::UserText { .. });
                        let after_work_group = b_idx > 0
                            && matches!(blocks[b_idx - 1], DisplayBlock::WorkGroup { .. })
                            && matches!(item, UiChatItem::AssistantText { .. });
                        let v_rb = cx.entity();
                        let v_full = cx.entity();
                        let v_img = cx.entity();
                        let subagent_task = find_subagent_task(&self.managed_tasks, item);
                        let item_el = render_chat_item(
                            ChatItemRenderParams {
                                item,
                                expanded,
                                full_output_expanded,
                                copied_key,
                                has_active_terminal,
                                now: frame_now,
                                subagent_task,
                                p,
                            },
                            ChatItemCallbacks {
                                on_toggle_expand: Rc::new(move |_, cx| {
                                    let id = cid.clone();
                                    v_exp.update(cx, |_, cx| {
                                        cx.emit(MessageStreamEvent::ToggleCard(id));
                                    });
                                }),
                                on_toggle_full_output: Rc::new(move |_, cx| {
                                    let key = full_key.clone();
                                    v_full.update(cx, |_, cx| {
                                        cx.emit(MessageStreamEvent::ToggleCard(key));
                                    });
                                }),
                                on_rollback: Rc::new(move |entry_id, _, cx| {
                                    v_rb.update(cx, |_, cx| {
                                        cx.emit(MessageStreamEvent::RollbackCheckpoint(entry_id));
                                    });
                                }),
                                on_preview_image: Rc::new(move |title, base64, _, cx| {
                                    v_img.update(cx, |_, cx| {
                                        cx.emit(MessageStreamEvent::PreviewImage { title, base64 });
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
                                expanded_cards,
                                active_focus: self.active_focus.as_deref(),
                                live_elapsed_secs: live_elapsed,
                                copied_key,
                                has_active_terminal,
                                now: frame_now,
                                managed_tasks: &self.managed_tasks,
                                p,
                            },
                            move |tid, _, cx| {
                                v_exp.update(cx, |_, cx| {
                                    cx.emit(MessageStreamEvent::ToggleCard(tid));
                                });
                            },
                            code_callbacks.clone(),
                        ));
                    }
                }
            }

            if is_running && !has_active_wg {
                let focus = self
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
                let label = format_running_focus(&focus);
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

        if let Some(err) = &self.status_error {
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

        let auto_following = self.is_running && !self.auto_scroll_locked;
        if auto_following {
            self.scroll_handle.scroll_to_bottom();
        }
        let max_y = self.scroll_handle.max_offset().y;
        let off_y = -self.scroll_handle.offset().y;
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
            let top_item = self.scroll_handle.top_item();
            let computed_dot = resolve_active_nav_dot(&nav_entries, top_item, !show_scroll_bottom);
            let active_dot = self.active_nav_dot.or(computed_dot);
            let rail = render_turn_nav_rail(
                &nav_entries,
                active_dot,
                p,
                move |dot_idx, block_idx, _, cx| {
                    view.update(cx, |this, cx| {
                        this.auto_scroll_locked = true;
                        this.scroll_handle.scroll_to_item(block_idx);
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
            wrapper = wrapper.child(render_scroll_bottom_button(p, move |_, cx| {
                v_btm.update(cx, |this, cx| {
                    this.auto_scroll_locked = false;
                    this.active_nav_dot = None;
                    this.scroll_handle.scroll_to_bottom();
                    cx.notify();
                });
            }));
        }
        wrapper
    }
}
