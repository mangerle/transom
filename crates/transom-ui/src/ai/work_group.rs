//! AI 中间工作过程（思考、过渡说明与工具调用）自动分组与可折叠进度块渲染。

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon, UiIcon};

use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::markdown_blocks::CodeBlockCallbacks;
use super::metrics::format_duration_zh;
use super::state::UiChatItem;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 消息流分组块：用户消息与最终答复直接展示，中间思考/过渡说明/工具调用聚合为可折叠工作块。
pub(super) enum DisplayBlock<'a> {
    Single(&'a UiChatItem),
    WorkGroup {
        id: String,
        items: Vec<&'a UiChatItem>,
        tool_steps: usize,
        ok_steps: usize,
        duration_ms: u64,
        in_progress: bool,
    },
}

pub(super) fn chat_item_id(item: &UiChatItem) -> &str {
    match item {
        UiChatItem::UserText { id, .. }
        | UiChatItem::Reasoning { id, .. }
        | UiChatItem::AssistantText { id, .. }
        | UiChatItem::ToolCard { id, .. } => id,
    }
}

/// 将扁平的消息流按对话轮次自动聚合为“中间工作块 + 最终输出”。
pub(super) fn group_chat_items(items: &[UiChatItem], is_running: bool) -> Vec<DisplayBlock<'_>> {
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < items.len() {
        if matches!(items[idx], UiChatItem::UserText { .. }) {
            blocks.push(DisplayBlock::Single(&items[idx]));
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < items.len() && !matches!(items[idx], UiChatItem::UserText { .. }) {
            idx += 1;
        }
        let slice = &items[start..idx];
        let at_tail = idx == items.len();
        let has_work = slice.iter().any(|it| {
            matches!(
                it,
                UiChatItem::Reasoning { .. } | UiChatItem::ToolCard { .. }
            )
        });
        if !has_work {
            for item in slice {
                blocks.push(DisplayBlock::Single(item));
            }
            continue;
        }
        let final_pos = slice
            .iter()
            .rposition(|it| {
                matches!(
                    it,
                    UiChatItem::AssistantText {
                        streaming: false,
                        ..
                    }
                )
            })
            .filter(|&pos| {
                !slice[pos + 1..].iter().any(|it| {
                    matches!(
                        it,
                        UiChatItem::Reasoning { .. } | UiChatItem::ToolCard { .. }
                    )
                })
            });
        let split_at = final_pos.unwrap_or(slice.len());
        let work_slice = &slice[..split_at];
        if !work_slice.is_empty() {
            let first_id = chat_item_id(&work_slice[0]);
            let mut tool_steps = 0usize;
            let mut ok_steps = 0usize;
            let mut duration_ms = 0u64;
            let mut work_items = Vec::with_capacity(work_slice.len());
            for item in work_slice {
                if let UiChatItem::ToolCard {
                    running,
                    ok,
                    duration_ms: d,
                    ..
                } = item
                {
                    tool_steps += 1;
                    if !*running && *ok {
                        ok_steps += 1;
                    }
                    duration_ms = duration_ms.saturating_add(*d);
                }
                work_items.push(item);
            }
            blocks.push(DisplayBlock::WorkGroup {
                id: format!("work-group-{first_id}"),
                items: work_items,
                tool_steps,
                ok_steps,
                duration_ms,
                in_progress: is_running && at_tail && final_pos.is_none(),
            });
        }
        if let Some(pos) = final_pos {
            blocks.push(DisplayBlock::Single(&slice[pos]));
        }
    }
    blocks
}

/// 渲染工作块渲染参数。
pub(super) struct WorkGroupParams<'a> {
    pub(super) id: &'a str,
    pub(super) items: &'a [&'a UiChatItem],
    pub(super) tool_steps: usize,
    pub(super) ok_steps: usize,
    pub(super) duration_ms: u64,
    pub(super) in_progress: bool,
    pub(super) expanded_cards: &'a HashSet<String>,
    pub(super) active_focus: Option<&'a str>,
    pub(super) live_elapsed_secs: Option<u64>,
    pub(super) copied_key: Option<&'a str>,
    pub(super) has_active_terminal: bool,
    pub(super) p: &'a Palette,
}

/// 渲染可折叠的中间工作块（运行中显示实时进度条，完成后折叠为单行摘要）。
pub(super) fn render_work_group(
    params: WorkGroupParams<'_>,
    on_toggle_id: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
    code_callbacks: CodeBlockCallbacks,
) -> AnyElement {
    let p = params.p;
    let expanded = params.expanded_cards.contains(params.id);
    let chevron = if expanded {
        UiIcon::CHEVRON_DOWN
    } else {
        UiIcon::CHEVRON_RIGHT
    };

    let header_text = if params.in_progress {
        let focus = params
            .active_focus
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| rust_i18n::t!("ai.working_default").to_string());
        let elapsed = params
            .live_elapsed_secs
            .map(|s| format_duration_zh(s.saturating_mul(1000)))
            .unwrap_or_default();
        if elapsed.is_empty() {
            rust_i18n::t!("ai.working_running", focus = focus).to_string()
        } else {
            format!(
                "{}  {elapsed}",
                rust_i18n::t!("ai.working_running", focus = focus)
            )
        }
    } else {
        let dur = if params.duration_ms >= 1000 {
            format_duration_zh(params.duration_ms)
        } else {
            "< 1s".to_owned()
        };
        rust_i18n::t!("ai.worked_for", duration = dur).to_string()
    };

    let gid = params.id.to_owned();
    let toggle_self = on_toggle_id.clone();
    let mut header = h_flex()
        .id(SharedString::from(format!("ai-wg-{}", params.id)))
        .w_full()
        .px(px(2.0))
        .py(px(3.0))
        .gap(rems(space::XS))
        .justify_between()
        .items_center()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, w, cx| toggle_self(gid.clone(), w, cx));

    let mut left_head = h_flex()
        .flex_1()
        .min_w_0()
        .gap(px(6.0))
        .items_center()
        .overflow_hidden();
    if params.in_progress {
        left_head = left_head.child(
            div()
                .flex_shrink_0()
                .text_color(p.primary)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small()),
        );
    }
    header = header.child(
        left_head
            .child(
                div()
                    .truncate()
                    .text_size(rems(typography::CAPTION))
                    .text_color(if params.in_progress {
                        p.foreground
                    } else {
                        p.text.muted
                    })
                    .child(header_text),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(p.text.muted)
                    .child(AppIconElement::new(AppIcon::Ui(chevron)).small()),
            ),
    );

    if params.tool_steps > 0 {
        let steps_label = rust_i18n::t!(
            "ai.work_steps_summary",
            done = params.ok_steps,
            total = params.tool_steps
        )
        .to_string();
        let (step_icon, step_color) = if params.in_progress {
            (UiIcon::REFRESH, p.primary)
        } else if params.ok_steps < params.tool_steps {
            (UiIcon::ERROR, p.danger)
        } else {
            (UiIcon::CHECK, p.success)
        };
        header = header.child(
            h_flex()
                .flex_shrink_0()
                .gap(px(4.0))
                .items_center()
                .child(
                    div()
                        .text_color(step_color)
                        .child(AppIconElement::new(AppIcon::Ui(step_icon)).small()),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(steps_label),
                ),
        );
    }

    let mut wrapper = v_flex().w_full().gap(px(4.0)).child(header);
    if expanded {
        let mut details = v_flex()
            .w_full()
            .ml(px(6.0))
            .pl(rems(space::SM))
            .py(px(2.0))
            .gap(px(4.0))
            .border_l_1()
            .border_color(p.divider(p.border));
        if let Some((wg_icon, wg_label)) = summarize_tool_activity(params.items) {
            details = details.child(
                h_flex()
                    .px(rems(space::XS))
                    .py(px(2.0))
                    .gap(px(6.0))
                    .items_center()
                    .child(
                        div()
                            .text_color(p.text.muted)
                            .child(AppIconElement::new(wg_icon).small()),
                    )
                    .child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(wg_label),
                    ),
            );
        }
        for item in params.items {
            let cid = chat_item_id(item).to_owned();
            let item_expanded = params.expanded_cards.contains(&cid);
            let full_key = format!("full-out-{cid}");
            let full_output_expanded = params.expanded_cards.contains(&full_key);
            let t = on_toggle_id.clone();
            let t_full = on_toggle_id.clone();
            details = details.child(render_chat_item(
                ChatItemRenderParams {
                    item,
                    expanded: item_expanded,
                    full_output_expanded,
                    copied_key: params.copied_key,
                    has_active_terminal: params.has_active_terminal,
                    p,
                },
                ChatItemCallbacks {
                    on_toggle_expand: Rc::new(move |w, cx| t(cid.clone(), w, cx)),
                    on_toggle_full_output: Rc::new(move |w, cx| t_full(full_key.clone(), w, cx)),
                    on_rollback: Rc::new(|_, _, _| {}),
                    code: code_callbacks.clone(),
                },
            ));
        }
        wrapper = wrapper.child(details);
    } else if params.in_progress {
        // 未手动展开工作块时，仍原地透出正在执行中的工具卡片（显示实时耗时与流式输出预览）
        let running_items: Vec<&UiChatItem> = params
            .items
            .iter()
            .copied()
            .filter(|it| matches!(it, UiChatItem::ToolCard { running: true, .. }))
            .collect();
        if !running_items.is_empty() {
            let mut live_list = v_flex()
                .w_full()
                .ml(px(6.0))
                .pl(rems(space::SM))
                .py(px(2.0))
                .gap(px(4.0))
                .border_l_1()
                .border_color(p.primary.opacity(0.4));
            for item in running_items {
                let cid = chat_item_id(item).to_owned();
                let item_expanded = params.expanded_cards.contains(&cid);
                let full_key = format!("full-out-{cid}");
                let full_output_expanded = params.expanded_cards.contains(&full_key);
                let t = on_toggle_id.clone();
                let t_full = on_toggle_id.clone();
                live_list = live_list.child(render_chat_item(
                    ChatItemRenderParams {
                        item,
                        expanded: item_expanded,
                        full_output_expanded,
                        copied_key: params.copied_key,
                        has_active_terminal: params.has_active_terminal,
                        p,
                    },
                    ChatItemCallbacks {
                        on_toggle_expand: Rc::new(move |w, cx| t(cid.clone(), w, cx)),
                        on_toggle_full_output: Rc::new(move |w, cx| {
                            t_full(full_key.clone(), w, cx)
                        }),
                        on_rollback: Rc::new(|_, _, _| {}),
                        code: code_callbacks.clone(),
                    },
                ));
            }
            wrapper = wrapper.child(live_list);
        }
    }
    wrapper.into_any_element()
}

fn summarize_tool_activity(items: &[&UiChatItem]) -> Option<(AppIcon, String)> {
    let mut has_cmd = false;
    let mut has_file = false;
    for item in items {
        if let UiChatItem::ToolCard { tool_name, .. } = item {
            match tool_name.as_str() {
                "remote_exec" | "remote_exec_bulk" | "container_ops" => has_cmd = true,
                "remote_fs" | "transfer_file" | "read_output" => has_file = true,
                _ => {}
            }
        }
    }
    match (has_file, has_cmd) {
        (true, true) => Some((
            AppIcon::Ui(UiIcon::LIST_FILES),
            rust_i18n::t!("ai.wg_read_and_ran").to_string(),
        )),
        (true, false) => Some((
            AppIcon::Ui(UiIcon::LIST_FILES),
            rust_i18n::t!("ai.wg_read_files").to_string(),
        )),
        (false, true) => Some((
            AppIcon::Domain(DomainIcon::TERMINAL),
            rust_i18n::t!("ai.wg_ran_commands").to_string(),
        )),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{DisplayBlock, group_chat_items};
    use crate::ai::state::UiChatItem;

    #[test]
    fn groups_reasoning_and_tools_before_final_answer() {
        let items = vec![
            UiChatItem::UserText {
                id: "u1".to_owned(),
                text: "检查负载".to_owned(),
                timestamp: None,
            },
            UiChatItem::Reasoning {
                id: "r1".to_owned(),
                text: "先执行 uptime".to_owned(),
                streaming: false,
            },
            UiChatItem::ToolCard {
                id: "t1".to_owned(),
                call_id: String::new(),
                running: false,
                tool_name: "remote_exec".to_owned(),
                host_label: "web-01".to_owned(),
                risk_label: "只读".to_owned(),
                summary: "uptime".to_owned(),
                output: "load 0.1".to_owned(),
                ok: true,
                duration_ms: 1200,
                diff_preview: None,
            },
            UiChatItem::AssistantText {
                id: "a1".to_owned(),
                text: "当前负载正常。".to_owned(),
                model: None,
                tokens_per_sec: None,
                streaming: false,
                cached_segments: None,
            },
        ];
        let blocks = group_chat_items(&items, false);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(
            blocks[0],
            DisplayBlock::Single(UiChatItem::UserText { .. })
        ));
        match &blocks[1] {
            DisplayBlock::WorkGroup {
                tool_steps,
                ok_steps,
                duration_ms,
                in_progress,
                ..
            } => {
                assert_eq!(*tool_steps, 1);
                assert_eq!(*ok_steps, 1);
                assert_eq!(*duration_ms, 1200);
                assert!(!*in_progress);
            }
            _ => panic!("期望中间过程聚合成 WorkGroup"),
        }
        assert!(matches!(
            blocks[2],
            DisplayBlock::Single(UiChatItem::AssistantText { .. })
        ));
    }
}
