//! AI 中间工作过程（思考、过渡说明与工具调用）自动分组与可折叠进度块渲染。

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::cards::{ChatItemCallbacks, ChatItemRenderParams, render_chat_item};
use super::markdown_blocks::CodeBlockCallbacks;
use super::metrics::format_duration_zh;
use super::state::UiChatItem;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 消息流分组块：仅保存消息区间，避免缓存持有可能失效的切片引用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DisplayBlock {
    Single(usize),
    WorkGroup {
        id: String,
        start: usize,
        end: usize,
        in_progress: bool,
    },
}

/// 消息分组缓存：只缓存稳定区间，流式文本更新不会使分组重新计算。
#[derive(Clone, Debug)]
pub(super) struct MessageBlockCache {
    pub(super) item_count: usize,
    pub(super) is_running: bool,
    pub(super) group_revision: u64,
    pub(super) blocks: Vec<DisplayBlock>,
}

/// 根据工具卡片匹配对应的子代理托管任务（用于实时进度流展示与追加/中断交互）。
pub(super) fn find_subagent_task<'a>(
    tasks: &'a [super::task_bar::UiManagedTask],
    item: &UiChatItem,
) -> Option<&'a super::task_bar::UiManagedTask> {
    let UiChatItem::ToolCard {
        tool_name, summary, ..
    } = item
    else {
        return None;
    };
    if tool_name != "delegate_task" {
        return None;
    }
    tasks
        .iter()
        .rev()
        .find(|t| t.dto.kind == "subagent" && t.dto.title == *summary)
}

/// 统一格式化运行态焦点文案，避免重复拼接“正在执行”前缀。
pub(super) fn format_running_focus(focus: &str) -> String {
    let trimmed = focus.trim();
    if trimmed.starts_with(rust_i18n::t!("ai.wg_running_fallback").as_ref())
        || trimmed.starts_with("Running ")
    {
        trimmed.to_owned()
    } else {
        rust_i18n::t!("ai.working_running", focus = trimmed).to_string()
    }
}

pub(super) fn chat_item_id(item: &UiChatItem) -> &str {
    match item {
        UiChatItem::UserText { id, .. }
        | UiChatItem::Reasoning { id, .. }
        | UiChatItem::AssistantText { id, .. }
        | UiChatItem::ToolCard { id, .. }
        | UiChatItem::AuditNotice { id, .. } => id,
    }
}

/// 将扁平的消息流按对话轮次自动聚合为“中间工作块 + 最终输出”。
pub(super) fn group_chat_items(items: &[UiChatItem], is_running: bool) -> Vec<DisplayBlock> {
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < items.len() {
        if matches!(items[idx], UiChatItem::UserText { .. }) {
            blocks.push(DisplayBlock::Single(idx));
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < items.len() && !matches!(items[idx], UiChatItem::UserText { .. }) {
            idx += 1;
        }
        let slice = &items[start..idx];
        let at_tail = idx == items.len();
        let has_work = slice.iter().any(|item| {
            matches!(
                item,
                UiChatItem::Reasoning { .. }
                    | UiChatItem::ToolCard { .. }
                    | UiChatItem::AuditNotice { .. }
            )
        });
        if !has_work {
            blocks.extend((start..idx).map(DisplayBlock::Single));
            continue;
        }
        let final_pos = slice
            .iter()
            .rposition(|item| {
                matches!(
                    item,
                    UiChatItem::AssistantText {
                        streaming: false,
                        ..
                    }
                )
            })
            .filter(|&pos| {
                !slice[pos + 1..].iter().any(|item| {
                    matches!(
                        item,
                        UiChatItem::Reasoning { .. }
                            | UiChatItem::ToolCard { .. }
                            | UiChatItem::AuditNotice { .. }
                    )
                })
            });
        let split_at = final_pos.unwrap_or(slice.len());
        if split_at > 0 {
            let first_id = chat_item_id(&items[start]);
            blocks.push(DisplayBlock::WorkGroup {
                id: format!("work-group-{first_id}"),
                start,
                end: start + split_at,
                in_progress: is_running && at_tail && final_pos.is_none(),
            });
        }
        if let Some(pos) = final_pos {
            blocks.push(DisplayBlock::Single(start + pos));
        }
    }
    blocks
}

/// 渲染工作块渲染参数。
pub(super) struct WorkGroupParams<'a> {
    pub(super) id: &'a str,
    pub(super) items: &'a [UiChatItem],
    pub(super) in_progress: bool,
    pub(super) expanded_cards: &'a HashSet<String>,
    pub(super) active_focus: Option<&'a str>,
    pub(super) live_elapsed_secs: Option<u64>,
    pub(super) copied_key: Option<&'a str>,
    pub(super) has_active_terminal: bool,
    pub(super) managed_tasks: &'a [super::task_bar::UiManagedTask],
    pub(super) p: &'a Palette,
}

/// 渲染可折叠的中间工作块（运行中显示实时进度条，完成后折叠为单行摘要）。
pub(super) fn render_work_group(
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
            let subagent_task = find_subagent_task(params.managed_tasks, item);
            details = details.child(render_chat_item(
                ChatItemRenderParams {
                    item,
                    expanded: item_expanded,
                    full_output_expanded,
                    copied_key: params.copied_key,
                    has_active_terminal: params.has_active_terminal,
                    subagent_task,
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
                        subagent_task,
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

use super::work_group_summary::{summarize_tool_activity, summarize_work_group_compact};

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
                revert_target_path: None,
                revert_backup_path: None,
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
        assert!(matches!(blocks[0], DisplayBlock::Single(0)));
        match &blocks[1] {
            DisplayBlock::WorkGroup { in_progress, .. } => {
                assert!(!*in_progress);
            }
            _ => panic!("期望中间过程聚合成 WorkGroup"),
        }
        assert!(matches!(blocks[2], DisplayBlock::Single(3)));
    }
}
