//! 选区状态管理与文本提取算法。

use super::TerminalEngine;
use alacritty_terminal::index::{Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};

impl TerminalEngine {
    /// 开始创建选区。
    pub fn start_selection(&mut self, ty: SelectionType, point: Point, side: Side) {
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    /// 更新选区终点。
    pub fn update_selection(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    /// 清空当前选区。
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// 提取选区内的纯文本。
    ///
    /// 块选（矩形列选）在此补齐每行到选区宽度：alacritty 会裁掉每段行尾空白，短行于是
    /// 返回空串，抠出来的各列整体左移错位——而"按列对齐"正是列选唯一的价值所在。
    /// 代价：宽度按字符数估算，选中区域含中文等双格字符时补齐会差半格，
    /// 内核返回的已是裁剪过的文本，原始列数无从还原。
    pub fn selection_text(&self) -> Option<String> {
        let text = self.term.selection_to_string()?;
        let Some(range) = self.selection_range() else {
            return Some(text);
        };
        if !range.is_block {
            return Some(text);
        }
        let width = range.end.column.0.saturating_sub(range.start.column.0) + 1;
        Some(
            text.lines()
                .map(|line| {
                    let deficit = width.saturating_sub(line.chars().count());
                    if deficit == 0 {
                        line.to_string()
                    } else {
                        format!("{line}{}", " ".repeat(deficit))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// 获取规范化后的选区范围。
    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.term))
    }
}
