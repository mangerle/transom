//! OSC 133 语义块布局计算与状态查询。

use super::TerminalEngine;
use crate::block::{BlockLayout, CommandBlock};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use transom_core::BlockViewSettings;

impl TerminalEngine {
    /// 当前设置下构建折叠行映射；`None` 表示走经典连续流。
    pub(crate) fn build_block_layout(
        &self,
        settings: Option<&BlockViewSettings>,
    ) -> Option<BlockLayout> {
        let settings = settings?;
        if !settings.enabled || self.blocks_saturated {
            return None;
        }
        // 备屏程序自己管理整屏坐标，重映射会破坏其鼠标与光标语义。
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }
        let grid = self.term.grid();
        Some(crate::block::build_layout(
            grid.history_size(),
            grid.screen_lines(),
            grid.display_offset(),
            &self.blocks,
            settings.style,
        ))
    }

    /// 调整自动折叠阈值（全局设置热更新时调用）。
    pub fn set_block_auto_collapse(&mut self, rows: usize) {
        self.blocks.set_auto_collapse_rows(rows);
        self.block_layout_dirty = true;
    }

    /// 获取所有命令块的迭代器。
    pub fn command_blocks(&self) -> impl Iterator<Item = &CommandBlock> {
        self.blocks.blocks()
    }

    /// 按 ID 检索命令块。
    pub fn command_block(&self, id: u64) -> Option<&CommandBlock> {
        self.blocks.block(id)
    }

    /// 切换指定块的折叠状态；返回是否命中。
    pub fn toggle_block(&mut self, id: u64) -> bool {
        let changed = self.blocks.toggle(id);
        if changed {
            self.block_layout_dirty = true;
        }
        changed
    }

    /// 显式设置指定块的折叠状态。
    pub fn set_block_collapsed(&mut self, id: u64, collapsed: bool) {
        self.blocks.set_collapsed(id, collapsed);
        self.block_layout_dirty = true;
    }

    /// 覆盖某绝对行的命令块（供鼠标悬浮与右键菜单定位）。
    pub fn command_block_at(&self, absolute_line: i64) -> Option<&CommandBlock> {
        self.blocks.block_at(absolute_line)
    }

    /// 命令块的纯净输出文本；行尾空白已裁剪。
    pub fn block_output_text(&self, id: u64) -> Option<String> {
        let block = self.blocks.block(id)?;
        self.read_absolute_lines(block.output_start_line, block.output_end_line)
    }

    /// 命令块对应的原始命令文本。
    pub fn block_command_text(&self, id: u64) -> Option<String> {
        self.blocks
            .block(id)
            .map(|block| block.command_text.clone())
    }

    /// 带耗时状态的 Markdown 代码块，供"复制为 Markdown"。
    pub fn block_markdown(&self, id: u64) -> Option<String> {
        let block = self.blocks.block(id)?;
        let output = self
            .read_absolute_lines(block.output_start_line, block.output_end_line)
            .unwrap_or_default();
        let command = if block.command_text.is_empty() {
            "(未知命令)".to_string()
        } else {
            block.command_text.clone()
        };
        let status = match block.exit_code {
            Some(code) => format!("退出码 {code}"),
            None => "退出码未知".to_string(),
        };
        let duration = block
            .duration_ms
            .map(|ms| format!(" · 耗时 {}", crate::block::format_duration(ms)))
            .unwrap_or_default();
        Some(format!(
            "```console\n$ {command}\n{output}```\n\n> {status}{duration}\n"
        ))
    }

    /// 读取一段绝对行范围；越出 `[0, history + screen)` 的部分跳过。
    fn read_absolute_lines(&self, start: i64, end: i64) -> Option<String> {
        if end < start {
            return Some(String::new());
        }
        let grid = self.term.grid();
        let history = grid.history_size() as i64;
        let screen = grid.screen_lines() as i64;
        if start < 0 || end >= history + screen {
            return None;
        }
        let cols = grid.columns();
        let mut text = String::new();
        for abs in start..=end {
            let line = Line((abs - history) as i32);
            let mut row = String::with_capacity(cols);
            for col in 0..cols {
                let cell = &grid[Point::new(line, Column(col))];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    || cell.c == '\0'
                {
                    continue;
                }
                row.push(cell.c);
            }
            text.push_str(row.trim_end());
            text.push('\n');
        }
        Some(text)
    }
}
