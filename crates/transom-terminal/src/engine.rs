use crate::block::{BlockLayout, BlockStore, CommandBlock};
use crate::event::TerminalEventProxy;
use crate::osc::{OscEvent, OscScanner};
use crate::renderer::DamageInfo;
use crate::search::SearchState;

/// 视图来不及取走时允许暂存的 OSC 事件上限。
const MAX_PENDING_OSC_EVENTS: usize = 64;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{
    CursorShape as AlacCursorShape, CursorStyle as AlacCursorStyle, Processor,
};
use std::time::Instant;
use transom_core::{BlockViewSettings, TerminalCursorShape, TerminalProfile, TerminalSize};

struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl TermDimensions {
    fn new(cols: usize, rows: usize) -> Self {
        Self { cols, rows }
    }
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }

    fn last_column(&self) -> Column {
        Column(self.cols.saturating_sub(1))
    }
}

pub struct TerminalEngine {
    pub(crate) term: Term<TerminalEventProxy>,
    parser: Processor,
    size: TerminalSize,
    /// 旁路 OSC 扫描器，用于识别 shell 集成的命令边界（见 [`crate::osc`]）。
    osc_scanner: OscScanner,
    /// 扫描器产出、等待视图取走的事件。
    osc_events: Vec<OscEvent>,
    /// 最近一次 `B`（提示符结束）时的光标位置：命令输入行的起点，
    /// 供 `C` 到达时按网格补采命令文本。
    command_input_point: Option<Point>,
    /// 全文检索状态（查询、命中快照与游标），行为见 [`crate::search`]。
    pub(crate) search: SearchState,
    /// OSC 133 命令块：采集、折叠状态与自动折叠阈值。
    blocks: BlockStore,
    /// 滚动缓冲容量；达到上限后旧行开始淘汰，块行号随之失去意义。
    scrollback_limit: usize,
    /// 滚动缓冲已写满：停止记录块，避免折叠到错误的行段。
    blocks_saturated: bool,
    /// 最近一次收到字节流的时刻，用于把重扫推迟到输出停歇之后。
    pub(crate) last_write: Option<Instant>,
    /// 缓冲区纪元：任何新写入或网格重建都会推进，用于判定命中快照是否失效。
    ///
    /// 之所以只记"变过"而不做行号平移：`Line` 是相对视口底部的编号，新行滚入时全部
    /// 命中行号同步 -1，而库外部拿不到绝对行数，平移无从校准（见 `search` 模块说明）。
    pub(crate) epoch: u64,
}

impl TerminalEngine {
    pub fn new(
        size: TerminalSize,
        profile: &TerminalProfile,
        event_tx: flume::Sender<alacritty_terminal::event::Event>,
    ) -> Self {
        let config = Config {
            scrolling_history: profile.scrollback,
            // 双击选词的分隔符由设置驱动；语义选择（`SelectionType::Semantic`）在仿真
            // 内核里解析，所以必须在这里传入，视图层拿不到第二套口径。
            semantic_escape_chars: profile.word_separators.clone(),
            default_cursor_style: AlacCursorStyle {
                shape: match profile.cursor_shape {
                    TerminalCursorShape::Block => AlacCursorShape::Block,
                    TerminalCursorShape::Bar => AlacCursorShape::Beam,
                    TerminalCursorShape::Underline => AlacCursorShape::Underline,
                },
                blinking: profile.cursor_blink,
            },
            ..Default::default()
        };
        let dimensions = TermDimensions::new(size.cols as usize, size.rows as usize);
        let term = Term::new(config, &dimensions, TerminalEventProxy::new(event_tx));
        Self {
            term,
            parser: Processor::new(),
            size,
            osc_scanner: OscScanner::default(),
            osc_events: Vec::new(),
            command_input_point: None,
            blocks: BlockStore::default(),
            scrollback_limit: profile.scrollback,
            blocks_saturated: profile.scrollback == 0,
            search: SearchState::default(),
            last_write: None,
            epoch: 0,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.last_write = Some(Instant::now());
        self.epoch = self.epoch.wrapping_add(1);
        // 先旁路扫 OSC，再把同一份字节切成若干段喂给内核：在每条 A/B/C/D
        // 标记处停下取光标，块边界才能落在标记发生的那一行。扫描器不消费
        // 输入，畸形序列也只影响通知，不影响屏幕内容。
        let scanned = self.osc_scanner.feed_with_offsets(bytes);
        let mut consumed = 0usize;
        for (offset, event) in &scanned {
            let end = (*offset + 1).min(bytes.len());
            if end > consumed {
                self.parser.advance(&mut self.term, &bytes[consumed..end]);
                consumed = end;
            }
            self.handle_marker(event);
        }
        if consumed < bytes.len() {
            self.parser.advance(&mut self.term, &bytes[consumed..]);
        }
        self.osc_events
            .extend(scanned.into_iter().map(|(_, event)| event));
        if self.osc_events.len() > MAX_PENDING_OSC_EVENTS {
            // 视图长时间没取走（例如标签被隐藏）时丢最旧的，绝不让队列无界增长。
            let drop = self.osc_events.len() - MAX_PENDING_OSC_EVENTS;
            self.osc_events.drain(..drop);
        }
        // 滚动缓冲写满后旧行开始淘汰，绝对行号整体平移；从此不再采集块，
        // 宁可退回经典视图，也不把错误行段标成某条命令的输出。
        if self.scrollback_limit > 0
            && self.term.grid().history_size() >= self.scrollback_limit
            && !self.blocks_saturated
        {
            log::debug!("滚动缓冲已达上限，停止采集语义块并回退到经典视图");
            self.blocks_saturated = true;
            self.blocks.clear();
        }
    }

    /// 单条 OSC 133 标记：按当前位置更新命令块。
    ///
    /// 调用时内核已消费到该标记所在字节，网格光标即标记发生时的位置。
    fn handle_marker(&mut self, event: &OscEvent) {
        match event {
            OscEvent::PromptStart => {
                let line = self.cursor_absolute_line();
                if !self.blocks_saturated {
                    self.blocks.on_prompt(line);
                }
            }
            OscEvent::CommandStart => {
                self.command_input_point = Some(self.term.grid().cursor.point);
                if !self.blocks_saturated {
                    self.blocks.on_command_start(self.cursor_absolute_line());
                }
            }
            OscEvent::PreExec { command } => {
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
                if !self.blocks_saturated {
                    self.blocks.on_finish(
                        *exit_code,
                        *duration_ms,
                        self.cursor_absolute_line(),
                        false,
                    );
                }
            }
            OscEvent::CommandRecorded { .. } | OscEvent::Notify { .. } => {}
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
    fn grid_command_text(&self) -> Option<String> {
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

    fn cursor_absolute_line(&self) -> i64 {
        self.absolute_line(self.term.grid().cursor.point.line)
    }

    fn absolute_line(&self, line: Line) -> i64 {
        self.term.grid().history_size() as i64 + i64::from(line.0)
    }

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
    }

    pub fn command_blocks(&self) -> impl Iterator<Item = &CommandBlock> {
        self.blocks.blocks()
    }

    pub fn command_block(&self, id: u64) -> Option<&CommandBlock> {
        self.blocks.block(id)
    }

    /// 切换指定块的折叠状态；返回是否命中。
    pub fn toggle_block(&mut self, id: u64) -> bool {
        self.blocks.toggle(id)
    }

    pub fn set_block_collapsed(&mut self, id: u64, collapsed: bool) {
        self.blocks.set_collapsed(id, collapsed);
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

    /// 取走并清空待处理的 OSC 事件。
    pub(crate) fn take_osc_events(&mut self) -> Vec<OscEvent> {
        std::mem::take(&mut self.osc_events)
    }

    pub fn resize(&mut self, size: TerminalSize) {
        let width = size.cols.max(1) as usize;
        let height = size.rows.max(1) as usize;
        if width == self.size.cols as usize && height == self.size.rows as usize {
            return;
        }
        self.size = TerminalSize::new(width as u16, height as u16);
        self.term.resize(TermDimensions::new(width, height));
        // 折行重排会让所有缓存行号失去意义，命令块一并清空。
        self.blocks.clear();
        self.blocks_saturated = self.scrollback_limit == 0;
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn size(&self) -> TerminalSize {
        self.size
    }

    pub fn term(&self) -> &Term<TerminalEventProxy> {
        &self.term
    }

    pub fn term_mut(&mut self) -> &mut Term<TerminalEventProxy> {
        &mut self.term
    }

    /// 取走自上次调用以来的脏行集合，并立即重置内核的损伤记录。
    ///
    /// 设计原理（Why）：内核的 damage 跟踪天然覆盖折行重排、滚动（display_offset
    /// 变化自动全量）、INSERT 模式与光标新旧位置；选区等用户态不包含在内，
    /// 而本渲染器把选区画成独立覆盖层、不进行缓存，因此恰好不需要它参与。
    /// 必须在持有引擎锁的渲染准备阶段调用，保证"上次取走之后"的窗口完整。
    pub(crate) fn take_damage(&mut self) -> DamageInfo {
        let damage = match self.term.damage() {
            TermDamage::Full => DamageInfo::Full,
            // 迭代器只产出已损伤行，且行号已换算为视口坐标（含 display_offset）。
            TermDamage::Partial(iter) => DamageInfo::Rows(iter.map(|line| line.line).collect()),
        };
        // damage() 只报告不清零：Full 标志与行级损伤都会粘滞，
        // 不立即重置的话脏行跟踪退化为每帧全量，缓存形同虚设。
        self.term.reset_damage();
        damage
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn grid(&self) -> &alacritty_terminal::grid::Grid<Cell> {
        self.term.grid()
    }

    pub fn scroll_display(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn start_selection(&mut self, ty: SelectionType, point: Point, side: Side) {
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// 选区文本。
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

    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.term))
    }

    pub fn cursor_point(&self) -> Point {
        self.term.grid().cursor.point
    }

    /// 返回网格坐标处单元格的 OSC 8 超链接 URI；无链接返回 None。
    ///
    /// 设计原理（Why）：URI 查询收敛在引擎侧，调用方只管把屏幕坐标换算成
    /// 网格坐标；返回拥有的 String，避免把网格借用泄漏到锁外。
    pub fn hyperlink_at(&self, point: Point) -> Option<String> {
        self.term.grid()[point.line][point.column]
            .hyperlink()
            .map(|link| link.uri().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::Line;

    /// shell 集成只给边界标记（绝大多数 bash/zsh 配置）时，命令文本按
    /// "提示符结束点到光标"的网格快照补采。
    #[test]
    fn captures_command_from_grid_between_prompt_and_cursor() {
        let (mut engine, _events) = engine(40, 3);
        engine.process(b"\x1b]133;A\x1b\\user@host:~$ \x1b]133;B\x1b\\");
        engine.process(b"git status");
        engine.process(b"\x1b]133;C\x1b\\");
        assert_eq!(
            captured(&engine),
            vec!["git status".to_string()],
            "应采到用户敲下的整行且不含提示符"
        );
    }

    /// 集成把命令文本直接上报（WezTerm 的 `E;<cmd>`）时以它为准：
    /// 网格快照可能被折行与重绘影响，上报值不会。
    #[test]
    fn prefers_command_text_reported_by_shell_integration() {
        let (mut engine, _events) = engine(40, 3);
        engine.process(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
        engine.process(b"echo hi\x1b]133;E;echo hi > /tmp/x\x1b\\");
        assert_eq!(
            captured(&engine),
            vec!["echo hi > /tmp/x".to_string()],
            "带参上报优先于网格快照"
        );
    }

    /// 单字符、以空格开头的行（`HIST_IGNORE_SPACE` 约定）都不该进历史。
    #[test]
    fn skips_short_and_opted_out_commands() {
        let (mut engine, _events) = engine(40, 3);
        engine.process(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
        engine.process(b"x\x1b]133;C\x1b\\");
        engine.process(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
        engine.process(b" secret-token abc\x1b]133;C\x1b\\");
        assert!(captured(&engine).is_empty(), "短行与刻意不留档的行都不记录");
    }

    /// 没有 shell 集成（从不发 `B`）时不能凭空产出历史：起点未知就放弃。
    #[test]
    fn captures_nothing_without_prompt_marker() {
        let (mut engine, _events) = engine(40, 3);
        engine.process(b"plain output without integration\x1b]133;C\x1b\\");
        assert!(captured(&engine).is_empty());
    }

    /// 一秒钟内跑完的小命令与提示符常挤在同一批字节里：按 OSC 字节偏移
    /// 分段推进后，命令文本、输出行与退出码都必须完整落在同一条块上。
    #[test]
    fn block_captures_fast_command_in_one_chunk() {
        let (mut engine, _events) = engine(40, 10);
        engine.process(
            b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\echo hi\r\n\x1b]133;C\x1b\\hi\r\n\x1b]133;D;0;12\x1b\\\x1b]133;A\x1b\\$ ",
        );
        let blocks: Vec<_> = engine.command_blocks().cloned().collect();
        assert_eq!(blocks.len(), 1, "应采集到一条命令块");
        let block = &blocks[0];
        assert_eq!(block.command_text, "echo hi");
        assert_eq!(block.exit_code, Some(0));
        assert_eq!(block.duration_ms, Some(12));
        assert!(block.output_rows() >= 1, "输出至少包含 hi 所在行");
        let output = engine.block_output_text(block.id).expect("块输出应可读取");
        assert!(output.contains("hi"), "输出: {output:?}");
        let markdown = engine.block_markdown(block.id).expect("Markdown 应可生成");
        assert!(markdown.starts_with("```console\n$ echo hi"));
        assert!(markdown.contains("退出码 0"));
    }

    /// 自动折叠阈值在收尾结算时生效，布局把输出压成横幅。
    #[test]
    fn block_auto_collapses_and_layout_shows_banner() {
        let (mut engine, _events) = engine(40, 10);
        engine.set_block_auto_collapse(1);
        engine.process(
            b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\seq 3\r\n\x1b]133;C\x1b\\1\r\n2\r\n3\r\n\x1b]133;D;0\x1b\\\x1b]133;A\x1b\\$ ",
        );
        let block = engine.command_blocks().next().cloned().expect("命令块");
        assert!(block.collapsed, "3 行输出应超过阈值 1 自动折叠");
        let settings = BlockViewSettings {
            enabled: true,
            style: transom_core::BlockViewStyle::Card,
            auto_collapse_rows: 1,
        };
        let layout = engine
            .build_block_layout(Some(&settings))
            .expect("启用后应有布局");
        assert!(
            layout
                .slots()
                .iter()
                .any(|slot| matches!(slot, crate::block::VisualSlot::Banner { .. })),
            "折叠块必须渲染为横幅"
        );
        // 关闭块视图后回到经典连续流。
        let disabled = BlockViewSettings {
            enabled: false,
            ..settings
        };
        assert!(engine.build_block_layout(Some(&disabled)).is_none());
    }

    fn captured(engine: &TerminalEngine) -> Vec<String> {
        engine
            .osc_events
            .iter()
            .filter_map(|event| match event {
                OscEvent::CommandRecorded { command } => Some(command.clone()),
                _ => None,
            })
            .collect()
    }
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    /// 用指定 profile 建引擎；分隔符行为测试需要非默认配置。
    fn engine_with_profile(
        profile: TerminalProfile,
    ) -> (
        TerminalEngine,
        flume::Receiver<alacritty_terminal::event::Event>,
    ) {
        let (tx, rx) = flume::bounded(16);
        (
            TerminalEngine::new(TerminalSize::new(20, 4), &profile, tx),
            rx,
        )
    }

    fn engine(
        cols: u16,
        rows: u16,
    ) -> (
        TerminalEngine,
        flume::Receiver<alacritty_terminal::event::Event>,
    ) {
        let (tx, rx) = flume::bounded(16);
        let profile = TerminalProfile::default();
        (
            TerminalEngine::new(TerminalSize::new(cols, rows), &profile, tx),
            rx,
        )
    }

    /// 损伤跟踪的取走-重置契约：脏行渲染依赖"取走即重置、新输入必损伤"。
    #[test]
    fn take_damage_reports_changes_and_resets() {
        let (mut engine, _rx) = engine(20, 4);
        // 初始状态必然全量损伤：首帧必须完整绘制。
        assert!(matches!(engine.take_damage(), DamageInfo::Full));
        // 无新输入时只剩光标行损伤（内核恒定标记当前光标）。
        match engine.take_damage() {
            DamageInfo::Rows(rows) => {
                assert!(!rows.is_empty(), "光标行应保持损伤");
                assert!(rows.len() <= 4, "视口外行号不应出现");
            }
            other => panic!("静止网格不应报全量损伤: {other:?}"),
        }
        // 新输入必须重新损伤其所在行。
        engine.process(b"hi");
        match engine.take_damage() {
            DamageInfo::Rows(rows) => assert!(rows.contains(&0), "新输入行应损伤: {rows:?}"),
            DamageInfo::Full => {}
        }
        // resize 折行重排后必须回到全量。
        engine.resize(TerminalSize::new(10, 4));
        assert!(matches!(engine.take_damage(), DamageInfo::Full));
    }

    /// OSC 8：链接元数据进入单元格，渲染与点击打开都以此为准。
    #[test]
    fn hyperlink_at_returns_osc8_uri() {
        let (mut engine, _rx) = engine(20, 4);
        engine.process(b"\x1b]8;;https://example.com\x1b\\home\x1b]8;;\x1b\\plain");
        let uri = engine
            .hyperlink_at(Point::new(Line(0), Column(0)))
            .expect("链接文本的首格应携带 URI");
        assert_eq!(uri, "https://example.com");
        assert!(
            engine
                .hyperlink_at(Point::new(Line(0), Column(6)))
                .is_none(),
            "链接关闭后的文本不应携带 URI"
        );
    }

    /// 分隔符设置必须真的改变双击选词边界。
    #[test]
    fn word_separators_drive_semantic_selection() {
        let (mut engine, _events) = engine_with_profile(TerminalProfile::default());
        engine.process(b"aa-bb cc");
        engine.start_selection(
            SelectionType::Semantic,
            Point::new(Line(0), Column(4)),
            Side::Left,
        );
        assert_eq!(
            engine.selection_text().as_deref(),
            Some("aa-bb"),
            "默认分隔符不含连字符，应整段选中"
        );

        let dashed = TerminalProfile {
            word_separators: "- \t".into(),
            ..TerminalProfile::default()
        };
        let (mut engine, _events) = engine_with_profile(dashed);
        engine.process(b"aa-bb cc");
        engine.start_selection(
            SelectionType::Semantic,
            Point::new(Line(0), Column(4)),
            Side::Left,
        );
        assert_eq!(
            engine.selection_text().as_deref(),
            Some("bb"),
            "把连字符与空格列为分隔符后只应选中后半段"
        );
    }

    /// 默认分隔符必须与仿真内核常量逐字一致，否则不改设置时的双击行为就会悄悄漂移。
    #[test]
    fn default_word_separators_match_kernel_constant() {
        assert_eq!(
            TerminalProfile::default().word_separators,
            alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS
        );
    }

    #[test]
    fn processes_plain_text() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"hello\r\nworld");
        let grid = engine.grid();
        assert_eq!(grid[Point::new(Line(0), Column(0))].c, 'h');
        assert_eq!(grid[Point::new(Line(0), Column(4))].c, 'o');
        assert_eq!(grid[Point::new(Line(1), Column(0))].c, 'w');
        assert_eq!(engine.cursor_point(), Point::new(Line(1), Column(5)));
    }

    #[test]
    fn parses_sgr_and_wide_char_cells() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"\x1b[31mX");
        engine.process("中".as_bytes());
        engine.process(b"a");

        let grid = engine.grid();
        let x = &grid[Point::new(Line(0), Column(0))];
        assert_eq!(x.c, 'X');
        assert_eq!(x.fg, Color::Named(NamedColor::Red));

        let wide = &grid[Point::new(Line(0), Column(1))];
        assert_eq!(wide.c, '中');
        assert!(wide.flags.contains(Flags::WIDE_CHAR));
        let spacer = &grid[Point::new(Line(0), Column(2))];
        assert!(spacer.flags.contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(grid[Point::new(Line(0), Column(3))].c, 'a');
    }

    #[test]
    fn resize_and_scrollback() {
        let (mut engine, _events) = engine(10, 2);
        engine.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        engine.resize(TerminalSize::new(20, 3));
        assert_eq!(engine.size(), TerminalSize::new(20, 3));
        assert_eq!(engine.grid().columns(), 20);
        assert_eq!(engine.grid().screen_lines(), 3);

        engine.scroll_display(Scroll::Top);
        assert!(engine.display_offset() > 0);
        engine.scroll_to_bottom();
        assert_eq!(engine.display_offset(), 0);
    }

    #[test]
    fn default_cursor_style_follows_profile() {
        let (tx, _rx) = flume::bounded(16);
        let profile = TerminalProfile {
            cursor_shape: TerminalCursorShape::Bar,
            cursor_blink: false,
            ..TerminalProfile::default()
        };
        let engine = TerminalEngine::new(TerminalSize::new(20, 4), &profile, tx);
        let style = engine.term().cursor_style();
        assert_eq!(style.shape, AlacCursorShape::Beam);
        assert!(!style.blinking);

        let (tx, _rx) = flume::bounded(16);
        let profile = TerminalProfile {
            cursor_shape: TerminalCursorShape::Underline,
            cursor_blink: true,
            ..TerminalProfile::default()
        };
        let engine = TerminalEngine::new(TerminalSize::new(20, 4), &profile, tx);
        let style = engine.term().cursor_style();
        assert_eq!(style.shape, AlacCursorShape::Underline);
        assert!(style.blinking);
    }

    #[test]
    fn selection_returns_text() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"hello world");
        engine.start_selection(
            SelectionType::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        engine.update_selection(Point::new(Line(0), Column(4)), Side::Right);
        assert_eq!(engine.selection_text().as_deref(), Some("hello"));
    }

    /// 矩形列选的价值就在于"只抠每行的同一几列"，因此选区文本必须逐行截取到列区间。
    #[test]
    fn block_selection_extracts_only_the_column_range() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"aaaaBBBBaaaa\r\nccccBBBBdddd\r\neeeeBBBBffff");
        engine.start_selection(
            SelectionType::Block,
            Point::new(Line(0), Column(4)),
            Side::Left,
        );
        engine.update_selection(Point::new(Line(2), Column(7)), Side::Right);
        let range = engine.selection_range().expect("块选应有选区");
        assert!(range.is_block, "选区必须标记为块，渲染层据此逐行截取");
        assert_eq!(engine.selection_text().as_deref(), Some("BBBB\nBBBB\nBBBB"));
        // 非矩形选择完全不受补齐影响。
        engine.start_selection(
            SelectionType::Simple,
            Point::new(Line(0), Column(4)),
            Side::Left,
        );
        engine.update_selection(Point::new(Line(0), Column(7)), Side::Right);
        assert_eq!(engine.selection_text().as_deref(), Some("BBBB"));
    }

    /// 短行落在列区间之外时必须补空格，否则抠出来的各列会左移错位。
    #[test]
    fn block_selection_pads_short_rows() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"aaaaBBBBaaaa\r\ncccc\r\neeeeBBBBffff");
        engine.start_selection(
            SelectionType::Block,
            Point::new(Line(0), Column(4)),
            Side::Left,
        );
        engine.update_selection(Point::new(Line(2), Column(7)), Side::Right);
        assert_eq!(engine.selection_text().as_deref(), Some("BBBB\n    \nBBBB"));
    }

    /// 锁定检索赖以成立的行号语义：`Line` 相对视口底部编号，新行滚入会让同一条内容
    /// 的行号整体 -1，因此命中快照必须随缓冲区写入失效，而不是平移复用。
    #[test]
    fn line_indices_are_relative_to_viewport_bottom() {
        let (mut engine, _events) = engine(20, 2);
        engine.process(b"A\r\nB\r\nC");
        assert_eq!(engine.grid()[Line(-1)][Column(0)].c, 'A');
        assert_eq!(engine.grid()[Line(0)][Column(0)].c, 'B');
        assert_eq!(engine.grid()[Line(1)][Column(0)].c, 'C');

        // 再滚入一行：同样的内容整体下移一个行号，A 退到更深的位置。
        engine.process(b"\r\nD");
        assert_eq!(engine.grid()[Line(-2)][Column(0)].c, 'A');
        assert_eq!(engine.grid()[Line(-1)][Column(0)].c, 'B');
        assert_eq!(engine.grid()[Line(0)][Column(0)].c, 'C');
        assert_eq!(engine.grid()[Line(1)][Column(0)].c, 'D');
    }

    /// 关键词模式按字面量匹配，正则模式按表达式匹配；中文与宽字符同样可检索。
    #[test]
    fn searches_literal_and_regex_queries() {
        let (mut engine, _events) = engine(20, 4);
        engine.process("alpha.bravo\r\n中文 错误码 404\r\ngamma".as_bytes());
        let now = Instant::now();

        engine.set_search_query("br.vo", false);
        engine.refresh_search(now);
        assert_eq!(
            engine.search_status().total,
            0,
            "字面量 br.vo 不应命中 bravo"
        );

        engine.set_search_query("br.vo", true);
        engine.refresh_search(now);
        assert_eq!(engine.search_status().total, 1, "正则 br.vo 应命中 bravo");

        engine.set_search_query("错误", false);
        engine.refresh_search(now);
        let hit = engine.search_select(true).expect("应命中中文关键词");
        assert_eq!(hit.start.line, Line(1));

        engine.set_search_query("\\d+", true);
        engine.refresh_search(now);
        assert_eq!(engine.search_status().total, 1);
    }

    /// 游标在首尾都要回绕，且当前命中变化后视线要跟到可见区。
    #[test]
    fn cursor_wraps_and_scrolls_history_into_view() {
        let (mut engine, _events) = engine(20, 2);
        engine.process(b"mark one\r\nfiller a\r\nfiller b\r\nmark two");
        engine.set_search_query("mark", false);
        engine.refresh_search(Instant::now());
        assert_eq!(engine.search_status().total, 2);

        // 命中按行号升序：第一条是已滚入历史的 `mark one`。
        let oldest = engine.search_select(true).unwrap();
        assert_eq!(oldest.start.line, Line(-2));
        assert!(
            engine.display_offset() > 0,
            "历史命中必须把显示偏移推上去才看得见"
        );

        let newest = engine.search_select(true).unwrap();
        assert_eq!(newest.start.line, Line(1));
        assert_eq!(engine.display_offset(), 0, "屏内命中不应再滚动");

        let wrapped = engine.search_select(true).unwrap();
        assert_eq!(wrapped, oldest, "到末尾应回绕到第一个");
        let back = engine.search_select(false).unwrap();
        assert_eq!(back, newest, "反向越过头部应回绕到最后一个");
    }

    /// 新写入使快照失效：失效期间不绘制高亮，冷却期内也不重复全缓冲扫描。
    #[test]
    fn output_invalidates_snapshot_until_rescan_cooldown_allows() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"needle in haystack");
        let t0 = Instant::now();
        engine.set_search_query("needle", false);
        engine.refresh_search(t0);
        assert!(engine.search_paint(40).is_some());

        engine.process(b"\r\nmore");
        // 输出仍在继续：既未停歇也在冷却期内，绝不重扫，因此不画任何可能指错位置的高亮。
        assert!(
            engine.search_paint(40).is_none(),
            "缓冲区变化后不得沿用旧行号"
        );
        engine.refresh_search(t0);
        assert!(engine.search_paint(40).is_none(), "冷却期内不重扫");
        engine.refresh_search(t0 + std::time::Duration::from_millis(400));
        assert!(engine.search_paint(40).is_some());
    }

    /// 非法正则只登记错误文本，不能影响既有渲染或让检索崩溃。
    #[test]
    fn invalid_query_keeps_engine_usable() {
        let (mut engine, _events) = engine(20, 4);
        engine.process(b"still works");
        engine.set_search_query("(unclosed", true);
        engine.refresh_search(Instant::now());
        let status = engine.search_status();
        assert!(status.error.is_some());
        assert!(engine.search_paint(40).is_none());
        assert!(engine.search_select(true).is_none());
        engine.clear_search();
        assert!(!engine.search_status().has_query);
    }
}
