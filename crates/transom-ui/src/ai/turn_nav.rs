//! 对话消息流右侧的用户提问圆点导航条：支持悬停预览当时提问内容，点击快速跳转到对应位置。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use super::state::UiChatItem;
use super::work_group::DisplayBlock;
use crate::palette::Palette;
use crate::theme::tokens::space;

/// 导航条中的单条用户提问锚点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TurnNavEntry {
    pub(super) block_idx: usize,
    pub(super) turn_no: usize,
    pub(super) preview: String,
}

fn compact_preview(raw: &str, max_chars: usize) -> String {
    let single_line: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = single_line.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// 从已分组的展示块中提取所有用户提问锚点。
pub(super) fn collect_turn_nav_entries(blocks: &[DisplayBlock<'_>]) -> Vec<TurnNavEntry> {
    let mut entries = Vec::new();
    for (block_idx, block) in blocks.iter().enumerate() {
        if let DisplayBlock::Single(UiChatItem::UserText { text, .. }) = block {
            let turn_no = entries.len() + 1;
            entries.push(TurnNavEntry {
                block_idx,
                turn_no,
                preview: compact_preview(text, 60),
            });
        }
    }
    entries
}

/// 渲染消息流右侧的纵向圆点快捷导航栏。
pub(super) fn render_turn_nav_rail(
    entries: &[TurnNavEntry],
    active_idx: Option<usize>,
    p: &Palette,
    on_jump: impl Fn(usize, usize, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let total = entries.len();
    let active = active_idx.unwrap_or_else(|| total.saturating_sub(1));
    let mut rail = v_flex()
        .w(px(20.0))
        .h_full()
        .py(rems(space::SM))
        .gap(px(6.0))
        .items_center()
        .justify_center()
        .flex_shrink_0();

    for (idx, entry) in entries.iter().enumerate() {
        let is_active = idx == active;
        let block_idx = entry.block_idx;
        let jump = on_jump.clone();
        let tip = format!("{}\n{} / {}", entry.preview, entry.turn_no, total);
        let dot_size = if is_active { px(7.0) } else { px(5.5) };
        let dot_color = if is_active { p.foreground } else { p.border };
        let hover_color = p.foreground;
        rail = rail.child(
            h_flex()
                .id(("ai-turn-dot", idx))
                .w(px(16.0))
                .h(px(12.0))
                .items_center()
                .justify_center()
                .cursor_pointer()
                .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                .on_click(move |_, w, cx| jump(idx, block_idx, w, cx))
                .child(
                    div()
                        .w(dot_size)
                        .h(dot_size)
                        .rounded_full()
                        .bg(dot_color)
                        .hover(move |s| s.bg(hover_color)),
                ),
        );
    }

    rail.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::collect_turn_nav_entries;
    use crate::ai::state::UiChatItem;
    use crate::ai::work_group::DisplayBlock;

    #[test]
    fn collects_user_turns_and_truncates_preview() {
        let u1 = UiChatItem::UserText {
            id: "u1".to_owned(),
            text: "检查磁盘\n并分析大文件".to_owned(),
        };
        let a1 = UiChatItem::AssistantText {
            id: "a1".to_owned(),
            text: "好的".to_owned(),
            model: None,
            tokens_per_sec: None,
            streaming: false,
        };
        let long_text =
            "请帮我排查线上服务器内存泄漏和数据库连接池耗尽的根本原因并给出完整方案，谢谢！"
                .repeat(2);
        let u2 = UiChatItem::UserText {
            id: "u2".to_owned(),
            text: long_text,
        };
        let blocks = vec![
            DisplayBlock::Single(&u1),
            DisplayBlock::Single(&a1),
            DisplayBlock::Single(&u2),
        ];
        let nav = collect_turn_nav_entries(&blocks);
        assert_eq!(nav.len(), 2);
        assert_eq!(nav[0].block_idx, 0);
        assert_eq!(nav[0].turn_no, 1);
        assert_eq!(nav[0].preview, "检查磁盘 并分析大文件");
        assert_eq!(nav[1].block_idx, 2);
        assert_eq!(nav[1].turn_no, 2);
        assert!(nav[1].preview.ends_with('…'));
    }
}
