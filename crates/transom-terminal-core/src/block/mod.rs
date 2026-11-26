//! OSC 133 语义块：命令块的采集、折叠状态与可见行布局。
//!
//! 设计原理（Why）：
//! - 不替换终端的字符网格：块只是网格之上的一层"行归属"标注，底层仍是标准
//!   VT 仿真，`vim`/`tmux`/`fzf` 的按键与屏幕行为完全不变（与 Warp 的自绘输入
//!   框路线相反）；
//! - 块的边界来自 shell 集成的 `A/B/C/D` 标记，行号用"距网格底部的偏移"
//!   表达，因此正常滚动与输出追加都不会让已完成的块错位；
//! - 折叠不删除内容，只改变可见行映射：展开/收起是毫秒级的重绘，不需要
//!   重启会话，也不影响选择、复制与历史缓冲。
//!
//! 代价与局限：
//! - 滚动缓冲写满并开始淘汰旧行后，绝对行号会整体平移；引擎在达到上限时
//!   清空块记录，宁可回到经典视图，也不把错误行段当成命令输出；
//! - 备屏（ALT screen）下不启用折叠：全屏程序自己管理整屏坐标，重映射会
//!   破坏其鼠标与光标语义。

use alacritty_terminal::index::Line;
use std::collections::VecDeque;
use transom_core::BlockViewStyle;

/// 内存中保留的命令块上限：只为交互服务，不承担历史归档。
pub const BLOCK_HISTORY_LIMIT: usize = 200;

/// 一条已完成（或仍在执行）的命令块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlock {
    pub id: u64,
    /// 提示符起始的绝对行号。
    pub prompt_line: i64,
    /// 命令输入所在的绝对行号。
    pub command_line: i64,
    /// 输出首行的绝对行号。
    pub output_start_line: i64,
    /// 输出末行的绝对行号；空输出时等于 `output_start_line - 1`。
    pub output_end_line: i64,
    pub command_text: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub collapsed: bool,
}

impl CommandBlock {
    /// 输出行数（含空白行），用于折叠横幅与自动折叠判定。
    pub fn output_rows(&self) -> usize {
        let rows = self
            .output_end_line
            .saturating_sub(self.output_start_line)
            .saturating_add(1);
        rows.max(0) as usize
    }
}

/// 正在执行、尚未收尾的命令块。
#[derive(Debug)]
struct OpenBlock {
    id: u64,
    prompt_line: i64,
    command_line: i64,
    output_start_line: i64,
    command_text: String,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
}

/// 命令块状态机：由仿真引擎在解析完每批输入后驱动。
///
/// 事件顺序与 shell 集成一致：`A` 提示符 → `B` 命令输入 → `C` 执行 →
/// `D` 结束 → 下一条 `A`。任何标记缺失都按"能确定多少记多少"降级，
/// 不猜测未知的边界。
#[derive(Debug, Default)]
pub struct BlockStore {
    blocks: VecDeque<CommandBlock>,
    next_id: u64,
    pending_prompt_line: Option<i64>,
    pending_command_line: Option<i64>,
    open: Option<OpenBlock>,
    auto_collapse_rows: usize,
}

impl BlockStore {
    /// 设置自动折叠阈值；`0` 表示从不自动折叠。
    pub fn set_auto_collapse_rows(&mut self, rows: usize) {
        self.auto_collapse_rows = rows;
    }

    /// 提示符开始（`A`）：结算上一条命令，并记下新提示符所在行。
    pub fn on_prompt(&mut self, abs_line: i64) {
        self.finalize(abs_line.saturating_sub(1));
        self.pending_prompt_line = Some(abs_line);
        self.pending_command_line = None;
    }

    /// 命令输入开始（`B`）。
    pub fn on_command_start(&mut self, abs_line: i64) {
        self.pending_command_line = Some(abs_line);
    }

    /// 命令即将执行（`C`/`E`）：开启新的命令块。
    pub fn on_preexec(&mut self, abs_line: i64, command: String) {
        // 上一条没有收到结束标记时，用本次输出起点作为它的终点，避免块重叠。
        self.finalize(abs_line.saturating_sub(1));
        let prompt_line = self.pending_prompt_line.take().unwrap_or(abs_line);
        let command_line = self.pending_command_line.take().unwrap_or(prompt_line);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.open = Some(OpenBlock {
            id,
            prompt_line,
            command_line,
            output_start_line: abs_line,
            command_text: command,
            exit_code: None,
            duration_ms: None,
        });
    }

    /// 命令结束（`D`）：记录退出码与耗时。
    ///
    /// `prompt_follows` 表示同一批字节里紧跟一个 `A`：此时真正的输出末行
    /// 要等提示符落位后才能确定，先不结算，交给 [`Self::on_prompt`]。
    pub fn on_finish(
        &mut self,
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
        abs_line: i64,
        prompt_follows: bool,
    ) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.exit_code = exit_code;
        open.duration_ms = duration_ms;
        if !prompt_follows {
            self.finalize(abs_line);
        }
    }

    fn finalize(&mut self, end_abs: i64) {
        let Some(open) = self.open.take() else {
            return;
        };
        let output_end_line = end_abs.max(open.output_start_line.saturating_sub(1));
        let rows = output_end_line
            .saturating_sub(open.output_start_line)
            .saturating_add(1)
            .max(0) as usize;
        let collapsed = self.auto_collapse_rows > 0 && rows > self.auto_collapse_rows;
        let block = CommandBlock {
            id: open.id,
            prompt_line: open.prompt_line,
            command_line: open.command_line,
            output_start_line: open.output_start_line,
            output_end_line,
            command_text: open.command_text,
            exit_code: open.exit_code,
            duration_ms: open.duration_ms,
            collapsed,
        };
        self.blocks.push_back(block);
        while self.blocks.len() > BLOCK_HISTORY_LIMIT {
            self.blocks.pop_front();
        }
    }

    /// 会话尺寸变化或滚动缓冲写满时清空：绝对行号已不可靠。
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.open = None;
        self.pending_prompt_line = None;
        self.pending_command_line = None;
    }

    pub fn blocks(&self) -> impl Iterator<Item = &CommandBlock> {
        self.blocks.iter()
    }

    pub fn block(&self, id: u64) -> Option<&CommandBlock> {
        self.blocks.iter().find(|block| block.id == id)
    }

    /// 查找位于指定绝对行之前的最近一条命令提示符绝对行号。
    pub fn previous_prompt_line(&self, current_abs_line: i64) -> Option<i64> {
        self.blocks
            .iter()
            .rev()
            .map(|b| b.prompt_line)
            .find(|&line| line < current_abs_line)
    }

    /// 查找位于指定绝对行之后的最近一条命令提示符绝对行号。
    pub fn next_prompt_line(&self, current_abs_line: i64) -> Option<i64> {
        self.blocks
            .iter()
            .map(|b| b.prompt_line)
            .find(|&line| line > current_abs_line)
    }

    /// 覆盖某绝对行的命令块（提示符、命令与输出行都算块内）。
    ///
    /// 空输出块的 `output_end_line` 位于输出起点之前，区间下界必须取
    /// `command_line`，否则悬浮命中会漏掉整条空输出命令。
    pub fn block_at(&self, abs_line: i64) -> Option<&CommandBlock> {
        self.blocks.iter().rev().find(|block| {
            let bottom = block.output_end_line.max(block.command_line);
            (block.prompt_line..=bottom).contains(&abs_line)
        })
    }

    /// 覆盖某绝对行的已折叠块（只有输出区间需要隐藏）。
    pub fn collapsed_at(&self, abs_line: i64) -> Option<&CommandBlock> {
        self.blocks.iter().rev().find(|block| {
            block.collapsed
                && block.output_end_line >= block.output_start_line
                && (block.output_start_line..=block.output_end_line).contains(&abs_line)
        })
    }

    /// 切换指定块的折叠状态，返回是否命中。
    pub fn toggle(&mut self, id: u64) -> bool {
        let Some(block) = self.blocks.iter_mut().find(|block| block.id == id) else {
            return false;
        };
        if block.output_end_line < block.output_start_line {
            return false;
        }
        block.collapsed = !block.collapsed;
        true
    }

    pub fn set_collapsed(&mut self, id: u64, collapsed: bool) {
        if let Some(block) = self.blocks.iter_mut().find(|block| block.id == id)
            && block.output_end_line >= block.output_start_line
        {
            block.collapsed = collapsed;
        }
    }
}

/// 可见行布局中的一个位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualSlot {
    /// 正常的网格行。
    Line(Line),
    /// 被折叠块的横幅；点击可展开。
    Banner {
        block_id: u64,
        label: String,
        failed: bool,
    },
    /// 历史不足时顶部留白。
    Blank,
}

/// 折叠后的可见行映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLayout {
    slots: Vec<VisualSlot>,
    history_size: i64,
}

impl BlockLayout {
    pub fn slots(&self) -> &[VisualSlot] {
        &self.slots
    }

    /// 指定可见行对应的网格行；横幅或留白返回 `None`。
    pub fn line_at(&self, row: usize) -> Option<Line> {
        match self.slots.get(row)? {
            VisualSlot::Line(line) => Some(*line),
            _ => None,
        }
    }

    /// 指定可见行是否为折叠横幅，返回块 ID。
    pub fn banner_at(&self, row: usize) -> Option<u64> {
        match self.slots.get(row)? {
            VisualSlot::Banner { block_id, .. } => Some(*block_id),
            _ => None,
        }
    }

    /// 网格行在可见区中的行号；被折叠或不在视口内返回 `None`。
    pub fn row_of_line(&self, line: Line) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| matches!(slot, VisualSlot::Line(candidate) if *candidate == line))
    }
}

/// 依据折叠状态构建可见行映射。
///
/// 从视口底部向上填充 `rows` 个位置：遇到已折叠块的输出区间时压缩为一条
/// 横幅，省出的空间继续向上取更早的历史行；历史不足时顶部留白。
/// 这样光标与最新输出始终贴底，折叠不会把正在输入的提示符推出屏幕。
pub fn build_layout(
    history_size: usize,
    screen_lines: usize,
    display_offset: usize,
    store: &BlockStore,
    style: BlockViewStyle,
) -> BlockLayout {
    let _ = style;
    let history = history_size as i64;
    let rows = screen_lines;
    let bottom_abs = history + rows as i64 - 1 - display_offset as i64;
    let mut reversed: Vec<VisualSlot> = Vec::with_capacity(rows + 4);
    let mut abs = bottom_abs;
    while reversed.len() < rows && abs >= 0 {
        if let Some(block) = store.collapsed_at(abs) {
            let start = block.output_start_line.max(0);
            reversed.push(VisualSlot::Banner {
                block_id: block.id,
                label: banner_label(block),
                failed: block.exit_code.is_some_and(|code| code != 0),
            });
            // 横幅占据块的输出起点位置，其余输出行全部跳过。
            abs = start - 1;
            continue;
        }
        reversed.push(VisualSlot::Line(Line((abs - history) as i32)));
        abs -= 1;
    }
    while reversed.len() < rows {
        reversed.push(VisualSlot::Blank);
    }
    reversed.reverse();
    BlockLayout {
        slots: reversed,
        history_size: history,
    }
}

/// 折叠横幅文案：行数、退出状态与耗时。
pub fn banner_label(block: &CommandBlock) -> String {
    let rows = block.output_rows();
    let status = match block.exit_code {
        Some(0) => rust_i18n::t!("terminal.block.status_success").to_string(),
        Some(code) => rust_i18n::t!("terminal.block.status_exit_code", code = code).to_string(),
        None => rust_i18n::t!("terminal.block.status_finished").to_string(),
    };
    let duration = block
        .duration_ms
        .map(format_duration)
        .map(|text| format!(" · {text}"))
        .unwrap_or_default();
    rust_i18n::t!(
        "terminal.block.banner",
        rows = rows,
        status = status,
        duration = duration
    )
    .to_string()
}

pub fn format_duration(millis: u64) -> String {
    if millis < 1_000 {
        format!("{millis}ms")
    } else if millis < 60_000 {
        format!("{:.1}s", millis as f64 / 1_000.0)
    } else {
        let seconds = millis / 1_000;
        format!("{}m{}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests;
