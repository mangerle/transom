//! 选区状态管理与文本提取算法。

use super::TerminalEngine;
use alacritty_terminal::index::{Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vi_mode::ViMotion;

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

    /// 切换键盘复制模式（`TermMode::VI`），返回切换后的状态。
    ///
    /// 复用内核自带的 vi 状态：光标位置、`vi_motion` 的折行/宽字符处理与
    /// 滚动都已在 alacritty 里实现，自己再写一套只会引入两套语义。
    pub fn toggle_copy_mode(&mut self) -> bool {
        self.term.toggle_vi_mode();
        // 复制光标的出现/消失不改变网格内容，内核不会报损伤；不提升为全量，
        // 旧帧的残留光标会一直留在屏幕上。
        self.force_full_damage = true;
        self.copy_mode_active()
    }

    /// 当前是否处于键盘复制模式。
    pub fn copy_mode_active(&self) -> bool {
        self.term.mode().contains(TermMode::VI)
    }

    /// 复制模式光标的网格坐标。
    pub fn vi_cursor_point(&self) -> Point {
        self.term.vi_mode_cursor.point
    }

    /// 按 vi 语义移动复制光标，并把视口滚动到光标所在行。
    ///
    /// `vi_motion` 自身只改坐标、不滚视口；不补这一步，光标一旦走出视口
    /// 就再也看不见，键盘移动会像"卡住了"。
    pub fn vi_motion(&mut self, motion: ViMotion) {
        self.term.vi_motion(motion);
        let point = self.term.vi_mode_cursor.point;
        self.term.scroll_to_point(point);
        // 与真实光标同理：复制光标移动不产生网格损伤，而光标是烘焙进行缓存
        // 的单元格反色/描边的，不强制全量就会在旧位置留下一块残影。
        self.force_full_damage = true;
    }

    /// 以给定锚点开始键盘选区（起点侧固定为 `Left`）。
    pub fn begin_keyboard_selection(&mut self, ty: SelectionType, anchor: Point) {
        self.term.selection = Some(Selection::new(ty, anchor, Side::Left));
    }

    /// 把键盘选区终点扩展到当前光标（终点侧取 `Right`，含光标下单元格）。
    pub fn extend_keyboard_selection(&mut self, point: Point) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
        }
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
