//! AI 中间工作过程纯分组模型与子代理任务匹配（零 GPUI 渲染依赖）。

use crate::app::ai::state::UiChatItem;

/// 消息流分组块：仅保存消息区间，避免缓存持有可能失效的切片引用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app::ai) enum DisplayBlock {
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
pub(in crate::app::ai) struct MessageBlockCache {
    pub(in crate::app::ai) item_count: usize,
    pub(in crate::app::ai) is_running: bool,
    pub(in crate::app::ai) group_revision: u64,
    pub(in crate::app::ai) blocks: Vec<DisplayBlock>,
}

/// 根据工具卡片的 `call_id` 精确匹配对应的子代理托管任务；若任务缺少 `source_call_id`（历史兼容）则回退按标题匹配。
pub(in crate::app::ai) fn find_subagent_task<'a>(
    tasks: &'a [crate::app::ai::task_bar::UiManagedTask],
    item: &UiChatItem,
) -> Option<&'a crate::app::ai::task_bar::UiManagedTask> {
    let UiChatItem::ToolCard {
        call_id,
        tool_name,
        summary,
        ..
    } = item
    else {
        return None;
    };
    if tool_name != "delegate_task" {
        return None;
    }
    if !call_id.is_empty()
        && let Some(matched) = tasks
            .iter()
            .rev()
            .find(|t| t.dto.kind == "subagent" && t.dto.source_call_id.as_deref() == Some(call_id))
    {
        return Some(matched);
    }
    tasks.iter().rev().find(|t| {
        t.dto.kind == "subagent" && t.dto.source_call_id.is_none() && t.dto.title == *summary
    })
}

/// 统一格式化运行态焦点文案，避免重复拼接“正在执行”前缀。
pub(in crate::app::ai) fn format_running_focus(focus: &str) -> String {
    let trimmed = focus.trim();
    if trimmed.starts_with(rust_i18n::t!("ai.wg_running_fallback").as_ref())
        || trimmed.starts_with("Running ")
    {
        trimmed.to_owned()
    } else {
        rust_i18n::t!("ai.working_running", focus = trimmed).to_string()
    }
}

pub(in crate::app::ai) fn chat_item_id(item: &UiChatItem) -> &str {
    match item {
        UiChatItem::UserText { id, .. }
        | UiChatItem::Reasoning { id, .. }
        | UiChatItem::AssistantText { id, .. }
        | UiChatItem::ToolCard { id, .. }
        | UiChatItem::AuditNotice { id, .. } => id,
    }
}

/// 将扁平的消息流按对话轮次自动聚合为“中间工作块 + 最终输出”。
pub(in crate::app::ai) fn group_chat_items(
    items: &[UiChatItem],
    is_running: bool,
) -> Vec<DisplayBlock> {
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

#[cfg(test)]
mod tests {
    use super::{DisplayBlock, find_subagent_task, group_chat_items};
    use crate::ai::state::UiChatItem;

    #[test]
    fn groups_reasoning_and_tools_before_final_answer() {
        let items = vec![
            UiChatItem::UserText {
                id: "u1".to_owned(),
                text: "检查负载".to_owned(),
                images: Vec::new(),
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

    #[test]
    fn matches_subagent_task_by_source_call_id_even_with_duplicate_titles() {
        use crate::ai::task_bar::UiManagedTask;
        use transom_core::{AiManagedTaskDto, AiManagedTaskStatus};

        let tasks = vec![
            UiManagedTask {
                dto: AiManagedTaskDto {
                    task_id: "task-1".to_owned(),
                    kind: "subagent".to_owned(),
                    title: "巡检节点".to_owned(),
                    target_label: "web-01".to_owned(),
                    status: AiManagedTaskStatus::Running,
                    started_at_ms: 1,
                    finished_at_ms: None,
                    exit_code: None,
                    source_call_id: Some("call-1".to_owned()),
                },
                log: String::new(),
            },
            UiManagedTask {
                dto: AiManagedTaskDto {
                    task_id: "task-2".to_owned(),
                    kind: "subagent".to_owned(),
                    title: "巡检节点".to_owned(),
                    target_label: "web-02".to_owned(),
                    status: AiManagedTaskStatus::Running,
                    started_at_ms: 2,
                    finished_at_ms: None,
                    exit_code: None,
                    source_call_id: Some("call-2".to_owned()),
                },
                log: String::new(),
            },
        ];

        let card_1 = UiChatItem::ToolCard {
            id: "t1".to_owned(),
            call_id: "call-1".to_owned(),
            running: true,
            tool_name: "delegate_task".to_owned(),
            host_label: "web-01".to_owned(),
            risk_label: "只读".to_owned(),
            summary: "巡检节点".to_owned(),
            output: String::new(),
            ok: true,
            duration_ms: 0,
            diff_preview: None,
            revert_target_path: None,
            revert_backup_path: None,
        };
        let matched = find_subagent_task(&tasks, &card_1).expect("应通过 call_id 精确匹配");
        assert_eq!(matched.dto.task_id, "task-1");
    }
}
