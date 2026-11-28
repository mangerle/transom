//! 流式输出末尾单块独立 GPUI 子视图：将高频 `TextDelta` / `ReasoningDelta` 重绘隔离在单条末尾消息内。
//!
//! 设计原理（Why）：
//! - 在长对话场景下，若每 25ms 的流式增量都触发整个 `MessageStreamView` 重绘，会连带重建前面上百条
//!   已定稿消息的 GPUI 元素树与布局；
//! - 将当前正在流式输出的末尾条目托管给独立的 [`StreamingTailView`] 实体后，延续同一条目的增量
//!   仅触发本子视图局部 `cx.notify()`，彻底避免历史块陪跑重绘。

use std::rc::Rc;

use gpui_kit::*;

use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::markdown_blocks::{CodeBlockCallbacks, StreamingMarkdownCache};
use super::message_stream::MessageStreamEvent;
use super::state::UiChatItem;
use crate::palette::Palette;
use crate::theme::tokens::space;

/// 流式末尾单块视图实体。
pub(in crate::app::ai) struct StreamingTailView {
    item: Option<UiChatItem>,
    after_work_group: bool,
    copied_key: Option<String>,
    has_active_terminal: bool,
    callbacks: Option<ChatItemCallbacks>,
}

impl StreamingTailView {
    pub(in crate::app::ai) fn new() -> Self {
        Self {
            item: None,
            after_work_group: false,
            copied_key: None,
            has_active_terminal: false,
            callbacks: None,
        }
    }

    /// 当前是否已挂载匹配类型的流式末尾条目。
    pub(in crate::app::ai) fn matches_streaming_kind(&self, is_reasoning: bool) -> bool {
        match (&self.item, is_reasoning) {
            (
                Some(UiChatItem::Reasoning {
                    streaming: true, ..
                }),
                true,
            )
            | (
                Some(UiChatItem::AssistantText {
                    streaming: true, ..
                }),
                false,
            ) => true,
            _ => false,
        }
    }

    /// 同步末尾流式条目与渲染元数据。
    pub(in crate::app::ai) fn sync(
        &mut self,
        item: Option<UiChatItem>,
        after_work_group: bool,
        copied_key: Option<String>,
        has_active_terminal: bool,
        callbacks: ChatItemCallbacks,
        cx: &mut Context<Self>,
    ) {
        self.item = item;
        self.after_work_group = after_work_group;
        self.copied_key = copied_key;
        self.has_active_terminal = has_active_terminal;
        self.callbacks = Some(callbacks);
        cx.notify();
    }

    /// 清空末尾流式条目（流式结束或切换会话时调用）。
    pub(in crate::app::ai) fn clear(&mut self, cx: &mut Context<Self>) {
        if self.item.is_some() {
            self.item = None;
            cx.notify();
        }
    }

    /// 仅向当前末尾流式条目追加文本增量并局部重绘本实体。
    pub(in crate::app::ai) fn append_delta(
        &mut self,
        is_reasoning: bool,
        delta: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if is_reasoning {
            if let Some(UiChatItem::Reasoning {
                text,
                streaming: true,
                ..
            }) = self.item.as_mut()
            {
                text.push_str(delta);
                cx.notify();
                return true;
            }
            return false;
        }
        if let Some(UiChatItem::AssistantText {
            text,
            streaming: true,
            cached_segments,
            ..
        }) = self.item.as_mut()
        {
            text.push_str(delta);
            cached_segments
                .get_or_insert_with(StreamingMarkdownCache::default)
                .update(text);
            cx.notify();
            return true;
        }
        false
    }
}

impl Render for StreamingTailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(item) = &self.item else {
            return div().into_any_element();
        };
        let Some(callbacks) = self.callbacks.clone() else {
            return div().into_any_element();
        };
        let p = Palette::from(cx);
        let el = render_chat_item(
            ChatItemRenderParams {
                item,
                expanded: false,
                full_output_expanded: false,
                copied_key: self.copied_key.as_deref(),
                has_active_terminal: self.has_active_terminal,
                now: chrono::Utc::now(),
                subagent_task: None,
                p: &p,
            },
            callbacks,
        );
        if self.after_work_group {
            div().w_full().pt(px(4.0)).child(el).into_any_element()
        } else if matches!(item, UiChatItem::UserText { .. }) {
            div()
                .w_full()
                .pt(rems(space::SM))
                .child(el)
                .into_any_element()
        } else {
            el
        }
    }
}

/// 构造供 [`StreamingTailView`] 转发事件到父视图 [`super::message_stream::MessageStreamView`] 的回调集合。
pub(in crate::app::ai) fn build_tail_callbacks(
    parent: Entity<super::message_stream::MessageStreamView>,
    code: CodeBlockCallbacks,
) -> ChatItemCallbacks {
    let p_exp = parent.clone();
    let p_full = parent.clone();
    let p_rb = parent.clone();
    let p_img = parent;
    ChatItemCallbacks {
        on_toggle_expand: Rc::new(move |_, cx| {
            let _ = &p_exp;
            let _ = cx;
        }),
        on_toggle_full_output: Rc::new(move |_, cx| {
            let _ = &p_full;
            let _ = cx;
        }),
        on_rollback: Rc::new(move |entry_id, _, cx| {
            p_rb.update(cx, |_, cx| {
                cx.emit(MessageStreamEvent::RollbackCheckpoint(entry_id));
            });
        }),
        on_preview_image: Rc::new(move |title, base64, _, cx| {
            p_img.update(cx, |_, cx| {
                cx.emit(MessageStreamEvent::PreviewImage { title, base64 });
            });
        }),
        code,
    }
}
