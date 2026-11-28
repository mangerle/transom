//! 独立的 AI 消息流 GPUI 实体：承载消息列表渲染、流式局部增量更新、滚动跟踪与导航点。
//!
//! 设计原理（Why）：
//! - 高频流式 `TextDelta` / `ReasoningDelta` 重绘隔离在本实体内，顶栏、抽屉与输入区不陪跑重绘；
//! - 单向数据流：父视图经 `sync_snapshot` / `apply_streaming_delta` 向下推送数据，
//!   交互经 [`MessageStreamEvent`] 向上冒泡，渲染链路严禁 `read_entity` 反向读取。

use std::time::Instant;

use gpui_kit::base::v_flex;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::*;
use transom_core::AiAppEvent;

use super::empty_state::render_empty_state;
use super::markdown_blocks::CodeBlockCallbacks;
use super::message_stream_render::{
    StreamScrollParams, build_code_callbacks, render_running_focus_row, render_single_stream_block,
    render_status_error_banner, wrap_stream_with_nav_and_scroll,
};
use super::message_stream_types::{MessageStreamEvent, MessageStreamSnapshot};
use super::state::{MessageStreamModel, UiChatItem};
use super::streaming_tail::{StreamingTailView, build_tail_callbacks};
use super::task_bar::UiManagedTask;
use super::turn_nav::collect_turn_nav_entries;
use super::work_group::{
    DisplayBlock, MessageBlockCache, WorkGroupParams, group_chat_items, render_work_group,
};
use crate::palette::Palette;
use crate::theme::tokens::space;

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
    streaming_tail: Entity<StreamingTailView>,
    pub(in crate::app::ai) scroll_handle: ScrollHandle,
    pub(in crate::app::ai) auto_scroll_locked: bool,
    pub(in crate::app::ai) active_nav_dot: Option<usize>,
}

impl EventEmitter<MessageStreamEvent> for MessageStreamView {}

impl MessageStreamView {
    pub(in crate::app::ai) fn new(cx: &mut Context<Self>) -> Self {
        let streaming_tail = cx.new(|_| StreamingTailView::new());
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
            streaming_tail,
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
        self.sync_streaming_tail(cx);
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
        self.sync_streaming_tail(cx);
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
        self.sync_streaming_tail(cx);
        cx.notify();
    }

    /// 清空全部消息流。
    pub(in crate::app::ai) fn clear_items(&mut self, cx: &mut Context<Self>) {
        self.model.clear();
        self.refresh_block_cache();
        self.sync_streaming_tail(cx);
        cx.notify();
    }

    /// 追加单条消息条目。
    pub(in crate::app::ai) fn push_item(&mut self, item: UiChatItem, cx: &mut Context<Self>) {
        self.model.push_item(item);
        self.refresh_block_cache();
        self.sync_streaming_tail(cx);
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
        self.sync_streaming_tail(cx);
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

    /// 仅原地追加流式文本/思考增量并局部重绘末尾子实体（零历史块重绘、零全量列表拷贝）。
    pub(in crate::app::ai) fn apply_streaming_delta(
        &mut self,
        is_reasoning: bool,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        self.model.apply_streaming_delta(is_reasoning, delta);
        if self
            .streaming_tail
            .read(cx)
            .matches_streaming_kind(is_reasoning)
        {
            let appended = self
                .streaming_tail
                .update(cx, |tail, cx| tail.append_delta(is_reasoning, delta, cx));
            if appended {
                if !self.auto_scroll_locked {
                    self.scroll_handle.scroll_to_bottom();
                }
                return;
            }
        }
        self.refresh_block_cache();
        self.sync_streaming_tail(cx);
        if !self.auto_scroll_locked {
            self.scroll_handle.scroll_to_bottom();
        }
        cx.notify();
    }

    /// 将当前末尾独立流式条目同步到 [`StreamingTailView`] 子实体；无独立流式条目时清空。
    fn sync_streaming_tail(&mut self, cx: &mut Context<Self>) {
        let Some(tail_block) = self.streaming_tail_block_info() else {
            self.streaming_tail.update(cx, |tail, cx| tail.clear(cx));
            return;
        };
        let (item_idx, after_work_group) = tail_block;
        let item = self.model.items()[item_idx].clone();
        let copied_key = self.copied_key.clone();
        let has_active_terminal = self.has_active_terminal;
        let code = self.cached_code_callbacks(cx);
        let callbacks = build_tail_callbacks(cx.entity(), code);
        self.streaming_tail.update(cx, |tail, cx| {
            tail.sync(
                Some(item),
                after_work_group,
                copied_key,
                has_active_terminal,
                callbacks,
                cx,
            );
        });
    }

    /// 若最后一个可见块是正在流式输出的独立条目，返回其 `(item_idx, after_work_group)`。
    fn streaming_tail_block_info(&self) -> Option<(usize, bool)> {
        let blocks = self.block_cache.as_ref()?.blocks.as_slice();
        let last_pos = blocks.len().checked_sub(1)?;
        let DisplayBlock::Single(item_idx) = blocks[last_pos] else {
            return None;
        };
        let item = self.model.items().get(item_idx)?;
        let is_streaming = matches!(
            item,
            UiChatItem::Reasoning {
                streaming: true,
                ..
            } | UiChatItem::AssistantText {
                streaming: true,
                ..
            }
        );
        if !is_streaming {
            return None;
        }
        let after_work_group =
            last_pos > 0 && matches!(blocks[last_pos - 1], DisplayBlock::WorkGroup { .. });
        Some((item_idx, after_work_group))
    }

    fn refresh_block_cache(&mut self) {
        let is_running = self.is_running;
        let item_count = self.model.item_count();
        let group_revision = self.model.group_revision();
        if (self.block_cache.as_ref())
            .is_some_and(|cache| cache.is_fresh(item_count, is_running, group_revision))
        {
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
        let cb = build_code_callbacks(cx.entity());
        self.code_callbacks = Some(cb.clone());
        cb
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
            let blocks = if let Some(cache) = (self.block_cache.as_ref())
                .filter(|cache| cache.is_fresh(items.len(), is_running, group_revision))
            {
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

            let has_streaming_tail = self.streaming_tail_block_info().is_some();
            let render_end = if has_streaming_tail {
                blocks.len().saturating_sub(1)
            } else {
                blocks.len()
            };

            for (b_idx, block) in blocks[..render_end].iter().enumerate().skip(render_start) {
                let v_exp = cx.entity();
                match block {
                    DisplayBlock::Single(item_index) => {
                        msg_list = msg_list.child(render_single_stream_block(
                            b_idx,
                            *item_index,
                            items,
                            blocks,
                            expanded_cards,
                            copied_key,
                            has_active_terminal,
                            frame_now,
                            &self.managed_tasks,
                            code_callbacks.clone(),
                            p,
                            v_exp,
                        ));
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

            if has_streaming_tail {
                msg_list = msg_list.child(self.streaming_tail.clone());
            }

            if is_running && !has_active_wg {
                msg_list = msg_list.child(render_running_focus_row(
                    self.active_focus.as_deref(),
                    live_elapsed,
                    p,
                ));
            }
        }

        if let Some(err) = &self.status_error {
            msg_list = msg_list.child(render_status_error_banner(err, p));
        }

        wrap_stream_with_nav_and_scroll(
            msg_list.into_any_element(),
            StreamScrollParams {
                nav_entries: &nav_entries,
                scroll_handle: &self.scroll_handle,
                is_running: self.is_running,
                auto_scroll_locked: self.auto_scroll_locked,
                active_nav_dot: self.active_nav_dot,
                p,
            },
            cx.entity(),
        )
    }
}
