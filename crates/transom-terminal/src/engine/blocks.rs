//! OSC 133 语义块布局计算与状态查询。

use super::TerminalEngine;
use crate::block::{BlockLayout, CommandBlock};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use std::time::{Duration, Instant};
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

    /// 获取当前视口顶部的绝对行号。
    pub fn current_display_abs_line(&self) -> i64 {
        let grid = self.term.grid();
        let history = grid.history_size() as i64;
        let offset = grid.display_offset() as i64;
        history.saturating_sub(offset)
    }

    /// 滚动使指定绝对行进入视口顶部。返回视口是否真的动了。
    ///
    /// 目标已经贴在视口顶、或缓冲区根本不够长时，滚动量会被夹成原值；必须如实报
    /// "没动"，否则调用方以为跳转成功，用户看到的就是按键失灵。
    pub fn scroll_to_abs_line(&mut self, abs_line: i64) -> bool {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let offset = grid.display_offset() as i32;
        let target = (history - abs_line as i32).clamp(0, history);
        if target == offset {
            return false;
        }
        self.term.scroll_display(Scroll::Delta(target - offset));
        true
    }

    /// 提示符跳跃的参考行。
    ///
    /// 滚动过就取视口顶行（"往上翻一屏，找上一条"）；贴在底部时取光标行——那时刻度
    /// 意义上的"上一条"是最近一条**已完成**命令的提示符，用视口顶行做基准会让短
    /// 缓冲里的第一次跳转直接判成"没有上一条"。
    fn navigation_anchor(&self) -> i64 {
        if self.display_offset() == 0 {
            self.cursor_absolute_line()
        } else {
            self.current_display_abs_line()
        }
    }

    /// 滚动到上一条命令提示符位置。返回视口是否动了。
    pub fn scroll_to_previous_prompt(&mut self) -> bool {
        let anchor = self.navigation_anchor();
        match self.blocks.previous_prompt_line(anchor) {
            Some(target) => self.scroll_to_abs_line(target),
            None => false,
        }
    }

    /// 滚动到下一条命令提示符位置。返回视口是否动了。
    ///
    /// 没有更靠后的记录块时回到实时提示符（缓冲区底部）：已经站在最后一条命令上
    /// 再按"下一条"，用户要的就是回到正在输入的那一行，原地不动只会让人以为按键失灵。
    pub fn scroll_to_next_prompt(&mut self) -> bool {
        let anchor = self.navigation_anchor();
        match self.blocks.next_prompt_line(anchor) {
            Some(target) => self.scroll_to_abs_line(target),
            None => {
                if self.display_offset() == 0 {
                    false
                } else {
                    self.scroll_to_bottom();
                    true
                }
            }
        }
    }

    /// 非 0 退出码命令块的锚点行（提示符行），以网格相对行号给出。
    ///
    /// 与检索命中同一口径（`Line` 相对视口底部，负值进历史），刻度带才能共用
    /// 同一条换算公式。每帧遍历上限内的块（`BLOCK_HISTORY_LIMIT` 条），成本与
    /// 检索刻度同量级。
    pub fn failure_lines(&self) -> Vec<Line> {
        if self.blocks_saturated {
            return Vec::new();
        }
        let history = self.term.grid().history_size() as i64;
        self.blocks
            .blocks()
            .filter(|block| block.exit_code.is_some_and(|code| code != 0))
            .map(|block| Line((block.prompt_line - history) as i32))
            .collect()
    }

    /// 把滚动条刻度比例换算成"离该比例最近的一条失败命令"并滚过去。
    ///
    /// 刻度带是按像素桶压缩过的近似位置，只能取最近邻而不做精确行匹配；桶宽远小于
    /// 一屏高度，跳过去的误差对"找最近那条报错"这个意图没有影响。
    ///
    /// 返回视口是否动了：目标已经在视口顶或缓冲区不够长时为 `false`。
    pub fn scroll_to_failure_near(&mut self, fraction: f32) -> bool {
        if self.blocks_saturated {
            return false;
        }
        let grid = self.term.grid();
        let history = grid.history_size() as i64;
        // 分母 = 历史行数 + 屏幕行数 - 1，即最末一行的绝对行号；
        // 与 `failure_ticks` 经由 `search_ticks` 得出的比例完全同口径，
        // 否则点击刻度与画出的刻度会错开一行。
        let span = (history + grid.screen_lines() as i64 - 1).max(1);
        let Some(target) = self
            .blocks
            .blocks()
            .filter(|block| block.exit_code.is_some_and(|code| code != 0))
            .map(|block| {
                let distance = (block.prompt_line as f32 / span as f32 - fraction).abs();
                (block.prompt_line, distance)
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(abs_line, _)| abs_line)
        else {
            return false;
        };
        self.scroll_to_abs_line(target);
        true
    }

    /// 失败命令在刻度带上的位置，取值 `0.0`（缓冲区顶部）到 `1.0`（底部）。
    pub fn failure_ticks(&self, tick_rows: usize) -> Vec<f32> {
        let grid = self.term.grid();
        let top = grid.topmost_line();
        let bottom = Line(grid.screen_lines() as i32 - 1);
        crate::search_ticks::build_ticks(
            self.failure_lines().iter().map(|line| line.0),
            top,
            bottom,
            tick_rows,
        )
    }

    /// 是否已收到过至少一条 OSC 133 命令边界标记（远端自带集成的证据）。
    pub fn shell_markers_seen(&self) -> bool {
        self.shell_markers_seen
    }

    /// 是否已经开始收到远端输出：Shell 至少画过一次屏，注入才有意义。
    pub fn output_started(&self) -> bool {
        self.last_write.is_some()
    }

    /// 输出是否已停歇满 `window`。提示符落位后屏幕静止，是最安全的写入时机。
    pub fn output_idle(&self, now: Instant, window: Duration) -> bool {
        self.last_write
            .is_some_and(|last| now.saturating_duration_since(last) >= window)
    }

    /// 是否处于备屏：全屏程序自己接管输入与坐标，探针写进去会被当成按键。
    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }
}
