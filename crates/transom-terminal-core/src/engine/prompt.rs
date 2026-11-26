//! Shell 提示符边界、输入行状态与命令块输入采集。
//!
//! 设计原理（Why）：目录快捷动作必须依赖与命令块完全相同的 OSC 133 事实来源。
//! 把状态推进和历史命令采集留在这里，避免终端引擎主文件继续膨胀，也保证
//! “提示符是否空闲”与“当前命令文本”不会各用一套口径。

use super::TerminalEngine;
use crate::osc::OscEvent;
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::cell::Flags;

/// Shell 输入行当前所处的阶段。
///
/// 设计原理（Why）：应用层需要判断能否代用户写入 `cd`，但仅看"没有前台命令"
/// 还不够；用户可能已经输入了半条命令。用单一枚举表达未知、空提示符、已编辑与
/// 命令运行四种状态，避免多个布尔字段组合出“正在运行但输入行为空”这类非法状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptInputState {
    /// 尚未收到足够的 Shell 集成标记，不能安全判断。
    Unknown,
    /// 已到达提示符，且本提示符下没有用户输入。
    Empty,
    /// 已到达提示符，但用户已经编辑过输入行。
    Edited,
    /// Shell 已提交命令，前台程序可能正在运行。
    Running,
}

impl TerminalEngine {
    /// 单条 OSC 133 标记：按当前位置更新命令块。
    ///
    /// 调用时内核已消费到该标记所在字节，网格光标即标记发生时的位置。
    pub(super) fn handle_marker(&mut self, event: &OscEvent) {
        // 任何一条命令边界标记都证明远端已经在发 133，注入决策据此一次定死。
        if matches!(
            event,
            OscEvent::PromptStart
                | OscEvent::CommandStart
                | OscEvent::PreExec { .. }
                | OscEvent::CommandFinish { .. }
        ) {
            self.shell_markers_seen = true;
        }
        match event {
            OscEvent::PromptStart => {
                self.prompt_input_state = PromptInputState::Empty;
                let line = self.cursor_absolute_line();
                if !self.blocks_saturated {
                    self.blocks.on_prompt(line);
                }
            }
            OscEvent::CommandStart => {
                // `B` 标记提示符绘制完毕，此时输入行尚未包含用户输入。
                self.prompt_input_state = PromptInputState::Empty;
                self.command_input_point = Some(self.term.grid().cursor.point);
                if !self.blocks_saturated {
                    self.blocks.on_command_start(self.cursor_absolute_line());
                }
            }
            OscEvent::PreExec { command } => {
                self.prompt_input_state = PromptInputState::Running;
                let text = match command {
                    Some(text) => text.trim().to_string(),
                    None => self.grid_command_text().unwrap_or_default(),
                };
                self.record_command(&text);
                if !self.blocks_saturated {
                    self.blocks.on_preexec(self.cursor_absolute_line(), text);
                }
            }
            OscEvent::CommandFinish {
                exit_code,
                duration_ms,
            } => {
                // `D` 只证明命令结束；下一条 `A` 到达前不能断言输入行已经安全。
                self.prompt_input_state = PromptInputState::Unknown;
                if !self.blocks_saturated {
                    self.blocks.on_finish(
                        *exit_code,
                        *duration_ms,
                        self.cursor_absolute_line(),
                        false,
                    );
                }
            }
            // 命令记录、cwd 上报、桌面通知、进度、剪贴板请求与集成握手都不改变
            // 网格上的块边界：握手只喂状态机，cwd 只改应用层目录状态，进度与
            // 剪贴板请求由视图层处理。
            OscEvent::CommandRecorded { .. }
            | OscEvent::Cwd { .. }
            | OscEvent::Notify { .. }
            | OscEvent::Progress { .. }
            | OscEvent::ClipboardStore { .. }
            | OscEvent::ShellReport { .. }
            | OscEvent::IntegrationReady { .. } => {}
        }
    }

    /// 记录一条可进入历史的命令；短行与刻意不留档的行跳过。
    fn record_command(&mut self, text: &str) {
        if text.chars().count() < 2 || text.starts_with(' ') {
            return;
        }
        self.osc_events.push(OscEvent::CommandRecorded {
            command: text.to_string(),
        });
    }

    /// 从记录的输入起点读到当前光标，拼出用户敲下的那一行。
    ///
    /// 折行的长命令按 `WRAPLINE` 标记继续拼接；宽字符的占位格跳过，
    /// 否则中文命令行会在每个汉字后多出一个空格。
    ///
    /// 除命令采集外，补全浮层的采纳校验也用它核对网格内容：
    /// 网格与本地按键跟踪一致（或只差回显尾部）时，替换词才是安全的。
    pub fn grid_command_text(&self) -> Option<String> {
        let start = self.command_input_point?;
        let grid = self.term.grid();
        let cursor = grid.cursor.point;
        // 起点已不在当前可见窗口（发生过清屏或大幅滚动）时放弃，不猜。
        if cursor.line < start.line {
            return None;
        }
        let cols = self.size.cols as usize;
        let mut text = String::new();
        let mut line = start.line;
        let mut column = start.column;
        loop {
            let last_column = if line == cursor.line {
                cursor.column
            } else {
                Column(cols.saturating_sub(1))
            };
            while column <= last_column {
                let cell = &grid[Point::new(line, column)];
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    && cell.c != '\0'
                {
                    text.push(cell.c);
                }
                column = Column(column.0.saturating_add(1));
            }
            if line == cursor.line {
                break;
            }
            // 只有上一行确实折到下一行才继续；否则说明换了行（例如已回车执行）。
            let wrapped = grid[Point::new(line, Column(cols.saturating_sub(1)))]
                .flags
                .intersects(Flags::WRAPLINE);
            if !wrapped {
                break;
            }
            line += 1;
            column = Column(0);
        }
        Some(text.trim_end().to_string())
    }

    /// 当前提示符输入状态；目录快捷动作据此决定能否注入命令。
    pub fn prompt_input_state(&self) -> PromptInputState {
        self.prompt_input_state
    }

    /// 记录用户已经编辑当前提示符的输入行。
    ///
    /// 输入一旦被编辑过，就不尝试猜测用户是否已经删空；保持 [`PromptInputState::Edited`]
    /// 直到下一条提示符标记到达，以保守策略避免把代执行命令拼到半条命令后面。
    pub fn note_prompt_input_edited(&mut self) {
        if self.prompt_input_state == PromptInputState::Empty {
            self.prompt_input_state = PromptInputState::Edited;
        }
    }

    /// 记录应用层已向当前提示符提交一条命令。
    ///
    /// 这会立即关闭后续目录快捷动作的入口，直到 Shell 再次上报提示符；
    /// 即使远端没有及时发送 `C` 标记，也不会把第二条 `cd` 连续写入同一输入行。
    pub fn mark_programmatic_command_submitted(&mut self) {
        self.prompt_input_state = PromptInputState::Running;
    }
}
