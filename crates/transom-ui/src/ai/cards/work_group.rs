//! AI 中间工作过程（思考、过渡说明与工具调用）自动分组与可折叠进度块渲染。

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::app::ai::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use crate::app::ai::markdown_blocks::CodeBlockCallbacks;
use crate::app::ai::metrics::format_duration_zh;
use crate::app::ai::state::UiChatItem;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

pub(in crate::app::ai) use super::work_group_model::{
    DisplayBlock, MessageBlockCache, chat_item_id, find_subagent_task, format_running_focus,
    group_chat_items,
};

/// 渲染工作块渲染参数。
pub(in crate::app::ai) struct WorkGroupParams<'a> {
    pub(in crate::app::ai) id: &'a str,
    pub(in crate::app::ai) items: &'a [UiChatItem],
    pub(in crate::app::ai) in_progress: bool,
    pub(in crate::app::ai) expanded_cards: &'a HashSet<String>,
    pub(in crate::app::ai) active_focus: Option<&'a str>,
    pub(in crate::app::ai) live_elapsed_secs: Option<u64>,
    pub(in crate::app::ai) copied_key: Option<&'a str>,
    pub(in crate::app::ai) has_active_terminal: bool,
    pub(in crate::app::ai) now: chrono::DateTime<chrono::Utc>,
    pub(in crate::app::ai) managed_tasks: &'a [crate::app::ai::task_bar::UiManagedTask],
    pub(in crate::app::ai) p: &'a Palette,
}

/// 渲染可折叠的中间工作块（运行中显示实时进度条，完成后折叠为单行摘要）。
pub(in crate::app::ai) fn render_work_group(
    params: WorkGroupParams<'_>,
    on_toggle_id: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
    code_callbacks: CodeBlockCallbacks,
) -> AnyElement {
    let p = params.p;
    let mut tool_steps = 0usize;
    let mut ok_steps = 0usize;
    let mut duration_ms = 0u64;
    for item in params.items {
        if let UiChatItem::ToolCard {
            running,
            ok,
            duration_ms: item_duration,
            ..
        } = item
        {
            tool_steps += 1;
            if !running && *ok {
                ok_steps += 1;
            }
            duration_ms = duration_ms.saturating_add(*item_duration);
        }
    }
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
        let base = format_running_focus(&focus);
        if elapsed.is_empty() {
            base
        } else {
            format!("{base}  {elapsed}")
        }
    } else {
        let dur = if duration_ms >= 1000 {
            format_duration_zh(duration_ms)
        } else {
            "< 1s".to_owned()
        };
        let base = rust_i18n::t!("ai.worked_for", duration = dur).to_string();
        let compact = summarize_work_group_compact(params.items);
        if compact.is_empty() {
            base
        } else {
            format!("{base} · {compact}")
        }
    };

    let gid = params.id.to_owned();
    let toggle_self = on_toggle_id.clone();
    let mut header = h_flex()
        .id(SharedString::from(format!("ai-wg-{}", params.id)))
        .w_full()
        .px(rems(space::SM))
        .py(px(5.0))
        .gap(rems(space::XS))
        .justify_between()
        .items_center()
        .rounded(p.radius.md)
        .bg(if params.in_progress {
            p.tone.primary_subtle
        } else {
            p.surface.ghost
        })
        .border_1()
        .border_color(if params.in_progress {
            p.primary.opacity(0.28)
        } else {
            p.stroke.faint
        })
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.faint))
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
    } else {
        let dot_color = if tool_steps > 0 && ok_steps < tool_steps {
            p.danger
        } else {
            p.success
        };
        left_head = left_head.child(
            div()
                .flex_shrink_0()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(dot_color),
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

    if tool_steps > 0 {
        let steps_label =
            rust_i18n::t!("ai.work_steps_summary", done = ok_steps, total = tool_steps).to_string();
        let (step_icon, step_color) = if params.in_progress {
            (UiIcon::REFRESH, p.primary)
        } else if ok_steps < tool_steps {
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
            .ml(px(8.0))
            .pl(rems(space::SM))
            .py(px(3.0))
            .gap(px(4.0))
            .border_l_2()
            .border_color(if params.in_progress {
                p.primary.opacity(0.32)
            } else {
                p.stroke.subtle
            });
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
            let subagent_task = find_subagent_task(params.managed_tasks, item);
            details = details.child(render_chat_item(
                ChatItemRenderParams {
                    item,
                    expanded: item_expanded,
                    full_output_expanded,
                    copied_key: params.copied_key,
                    has_active_terminal: params.has_active_terminal,
                    now: params.now,
                    subagent_task,
                    p,
                },
                ChatItemCallbacks {
                    on_toggle_expand: Rc::new(move |w, cx| t(cid.clone(), w, cx)),
                    on_toggle_full_output: Rc::new(move |w, cx| t_full(full_key.clone(), w, cx)),
                    on_rollback: Rc::new(|_, _, _| {}),
                    on_preview_image: Rc::new(|_, _, _, _| {}),
                    code: code_callbacks.clone(),
                },
            ));
        }
        wrapper = wrapper.child(details);
    } else {
        // 未手动展开工作块时，原地透出正在执行中的工具卡片，以及所有子代理（delegate_task）卡片，
        // 避免多个并行或接力执行的子 Agent 在完成后缩进折叠块导致只能看到最后一个。
        let surfaced_items: Vec<&UiChatItem> = params
            .items
            .iter()
            .filter(|it| match it {
                UiChatItem::ToolCard {
                    running, tool_name, ..
                } => *running || tool_name == "delegate_task",
                _ => false,
            })
            .collect();
        if !surfaced_items.is_empty() {
            let mut live_list = v_flex()
                .w_full()
                .ml(px(6.0))
                .pl(rems(space::SM))
                .py(px(2.0))
                .gap(px(4.0))
                .border_l_1()
                .border_color(p.primary.opacity(0.4));
            for item in surfaced_items {
                let cid = chat_item_id(item).to_owned();
                let item_expanded = params.expanded_cards.contains(&cid);
                let full_key = format!("full-out-{cid}");
                let full_output_expanded = params.expanded_cards.contains(&full_key);
                let t = on_toggle_id.clone();
                let t_full = on_toggle_id.clone();
                let subagent_task = find_subagent_task(params.managed_tasks, item);
                live_list = live_list.child(render_chat_item(
                    ChatItemRenderParams {
                        item,
                        expanded: item_expanded,
                        full_output_expanded,
                        copied_key: params.copied_key,
                        has_active_terminal: params.has_active_terminal,
                        now: params.now,
                        subagent_task,
                        p,
                    },
                    ChatItemCallbacks {
                        on_toggle_expand: Rc::new(move |w, cx| t(cid.clone(), w, cx)),
                        on_toggle_full_output: Rc::new(move |w, cx| {
                            t_full(full_key.clone(), w, cx)
                        }),
                        on_rollback: Rc::new(|_, _, _| {}),
                        on_preview_image: Rc::new(|_, _, _, _| {}),
                        code: code_callbacks.clone(),
                    },
                ));
            }
            wrapper = wrapper.child(live_list);
        }
    }
    wrapper.into_any_element()
}

use crate::app::ai::work_group_summary::{summarize_tool_activity, summarize_work_group_compact};
