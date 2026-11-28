//! AI 消息流辅助渲染组件：运行态焦点行与错误提示横幅。

use std::collections::HashSet;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::markdown_blocks::CodeBlockCallbacks;
use super::message_stream::MessageStreamView;
use super::metrics::format_duration_zh;
use super::plan_card::render_scroll_bottom_button;
use super::state::UiChatItem;
use super::task_bar::UiManagedTask;
use super::turn_nav::{TurnNavEntry, render_turn_nav_rail, resolve_active_nav_dot};
use super::work_group::{DisplayBlock, chat_item_id, find_subagent_task, format_running_focus};
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

use super::message_stream_types::MessageStreamEvent;

/// 渲染消息流中的单条独立条目并应用组间间距。
#[allow(clippy::too_many_arguments)]
pub(in crate::app::ai) fn render_single_stream_block(
    b_idx: usize,
    item_index: usize,
    items: &[UiChatItem],
    blocks: &[DisplayBlock],
    expanded_cards: &HashSet<String>,
    copied_key: Option<&str>,
    has_active_terminal: bool,
    now: DateTime<Utc>,
    managed_tasks: &[UiManagedTask],
    code_callbacks: CodeBlockCallbacks,
    p: &Palette,
    view: Entity<MessageStreamView>,
) -> AnyElement {
    let item = &items[item_index];
    let cid = chat_item_id(item).to_owned();
    let expanded = expanded_cards.contains(&cid);
    let full_key = format!("full-out-{cid}");
    let full_output_expanded = expanded_cards.contains(&full_key);
    let is_follow_user = b_idx > 0 && matches!(item, UiChatItem::UserText { .. });
    let after_work_group = b_idx > 0
        && matches!(blocks[b_idx - 1], DisplayBlock::WorkGroup { .. })
        && matches!(item, UiChatItem::AssistantText { .. });
    let v_exp = view.clone();
    let v_full = view.clone();
    let v_rb = view.clone();
    let v_img = view;
    let subagent_task = find_subagent_task(managed_tasks, item);
    let item_el = render_chat_item(
        ChatItemRenderParams {
            item,
            expanded,
            full_output_expanded,
            copied_key,
            has_active_terminal,
            now,
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
            code: code_callbacks,
        },
    );
    if is_follow_user {
        div()
            .w_full()
            .pt(rems(space::SM))
            .child(item_el)
            .into_any_element()
    } else if after_work_group {
        div().w_full().pt(px(4.0)).child(item_el).into_any_element()
    } else {
        item_el
    }
}

/// 构造供消息流代码块与工具卡片复用、向上转发事件到 [`MessageStreamView`] 的回调集合。
pub(in crate::app::ai) fn build_code_callbacks(
    view: Entity<MessageStreamView>,
) -> CodeBlockCallbacks {
    let v_copy = view.clone();
    let v_send = view.clone();
    let v_snip = view.clone();
    let v_rev = view.clone();
    let v_jump = view.clone();
    let v_sub = view;
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

/// 渲染底部运行态焦点与耗时行。
pub(in crate::app::ai) fn render_running_focus_row(
    active_focus: Option<&str>,
    live_elapsed: Option<u64>,
    p: &Palette,
) -> AnyElement {
    let focus = active_focus
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| rust_i18n::t!("ai.working_default").to_string());
    let elapsed_str = live_elapsed
        .filter(|&s| s > 0)
        .map(|s| format_duration_zh(s.saturating_mul(1000)))
        .unwrap_or_default();
    let label = format_running_focus(&focus);
    let mut row = h_flex()
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
    row.into_any_element()
}

/// 渲染消息流底部的错误提示横幅。
pub(in crate::app::ai) fn render_status_error_banner(err: &str, p: &Palette) -> AnyElement {
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
                .child(err.to_owned()),
        )
        .into_any_element()
}

/// 消息流滚动容器的装配参数。
pub(in crate::app::ai) struct StreamScrollParams<'a> {
    pub(in crate::app::ai) nav_entries: &'a [TurnNavEntry],
    pub(in crate::app::ai) scroll_handle: &'a ScrollHandle,
    pub(in crate::app::ai) is_running: bool,
    pub(in crate::app::ai) auto_scroll_locked: bool,
    pub(in crate::app::ai) active_nav_dot: Option<usize>,
    pub(in crate::app::ai) p: &'a Palette,
}

/// 将消息流滚动列表与左侧轮次导航导轨及悬浮「回到底部」按钮组合为完整容器。
pub(in crate::app::ai) fn wrap_stream_with_nav_and_scroll(
    msg_list: AnyElement,
    params: StreamScrollParams<'_>,
    view: Entity<MessageStreamView>,
) -> Div {
    let StreamScrollParams {
        nav_entries,
        scroll_handle,
        is_running,
        auto_scroll_locked,
        active_nav_dot,
        p,
    } = params;
    let auto_following = is_running && !auto_scroll_locked;
    if auto_following {
        scroll_handle.scroll_to_bottom();
    }
    let max_y = scroll_handle.max_offset().y;
    let off_y = -scroll_handle.offset().y;
    let show_scroll_bottom = !auto_following && max_y > px(40.0) && (max_y - off_y) > px(40.0);

    let mut body_row = h_flex()
        .flex_1()
        .w_full()
        .h_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden();
    if !nav_entries.is_empty() {
        let top_item = scroll_handle.top_item();
        let computed_dot = resolve_active_nav_dot(nav_entries, top_item, !show_scroll_bottom);
        let active_dot = active_nav_dot.or(computed_dot);
        let v_nav = view.clone();
        let rail = render_turn_nav_rail(
            nav_entries,
            active_dot,
            p,
            move |dot_idx, block_idx, _, cx| {
                v_nav.update(cx, |this, cx| {
                    this.jump_to_nav_dot(dot_idx, block_idx, cx);
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
        wrapper = wrapper.child(render_scroll_bottom_button(p, move |_, cx| {
            view.update(cx, |this, cx| {
                this.reset_and_scroll_bottom(cx);
            });
        }));
    }
    wrapper
}

impl MessageStreamView {
    /// 显式解锁自动滚底并滚动到消息流底部（例如发送新提问时）。
    pub(in crate::app::ai) fn reset_and_scroll_bottom(&mut self, cx: &mut Context<Self>) {
        self.auto_scroll_locked = false;
        self.active_nav_dot = None;
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    /// 跳转到指定导航点对应的消息块并锁定自动滚底。
    pub(in crate::app::ai) fn jump_to_nav_dot(
        &mut self,
        dot_idx: usize,
        block_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.auto_scroll_locked = true;
        self.scroll_handle.scroll_to_item(block_idx);
        self.active_nav_dot = Some(dot_idx);
        cx.notify();
    }
}
