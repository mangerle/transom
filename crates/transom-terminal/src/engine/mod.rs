use crate::block::BlockStore;
use crate::event::TerminalEventProxy;
use crate::kitty_graphics::{GraphicsPlacement, GraphicsStore, KittyEvent, KittyParser};
use crate::osc::{OscEvent, OscScanner};
use crate::renderer::DamageInfo;
use crate::search::SearchState;

mod blocks;
mod selection;

#[cfg(test)]
mod tests;

/// 视图来不及取走时允许暂存的 OSC 事件上限。
const MAX_PENDING_OSC_EVENTS: usize = 64;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{
    CursorShape as AlacCursorShape, CursorStyle as AlacCursorStyle, Processor,
};
use std::time::Instant;
use transom_core::{TerminalCursorShape, TerminalProfile, TerminalSize};

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
    /// Kitty 图形协议解析器与已解码图像仓库。
    kitty: KittyParser,
    graphics: GraphicsStore,
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
            kitty: KittyParser::default(),
            graphics: GraphicsStore::default(),
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
        let kitty = self.kitty.feed_with_offsets(bytes);
        enum Marker {
            Osc(OscEvent),
            Kitty(KittyEvent),
        }
        let mut markers: Vec<(usize, Marker)> = Vec::with_capacity(scanned.len() + kitty.len());
        markers.extend(scanned.into_iter().map(|(o, e)| (o, Marker::Osc(e))));
        markers.extend(kitty.into_iter().map(|(o, e)| (o, Marker::Kitty(e))));
        markers.sort_by_key(|(offset, _)| *offset);
        let mut consumed = 0usize;
        let mut osc_events = Vec::with_capacity(markers.len());
        for (offset, marker) in markers {
            let end = (offset + 1).min(bytes.len());
            if end > consumed {
                self.parser.advance(&mut self.term, &bytes[consumed..end]);
                consumed = end;
            }
            match marker {
                Marker::Osc(event) => {
                    self.handle_marker(&event);
                    osc_events.push(event);
                }
                Marker::Kitty(event) => self.apply_kitty(event),
            }
        }
        if consumed < bytes.len() {
            self.parser.advance(&mut self.term, &bytes[consumed..]);
        }
        self.osc_events.extend(osc_events);
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
            self.graphics.clear();
        }
        let grid = self.term.grid();
        self.graphics
            .retain_in_buffer(grid.history_size(), grid.screen_lines());
    }

    /// 应用一条 Kitty 图像事件：存入仓库并按当前光标位置放置。
    fn apply_kitty(&mut self, event: KittyEvent) {
        let cursor = self.term.grid().cursor.point;
        let absolute = self.absolute_line(cursor.line);
        match event {
            KittyEvent::Transmit {
                id,
                image,
                width,
                height,
                cols,
                rows,
                display,
                x,
                y,
            } => {
                self.graphics.store(id, image, width, height);
                if display {
                    self.graphics.place(crate::kitty_graphics::PlaceRequest {
                        image_id: id,
                        absolute_line: absolute,
                        col: cursor.column.0,
                        cols,
                        rows,
                        x,
                        y,
                    });
                }
            }
            KittyEvent::Place {
                id,
                cols,
                rows,
                x,
                y,
            } => self.graphics.place(crate::kitty_graphics::PlaceRequest {
                image_id: id,
                absolute_line: absolute,
                col: cursor.column.0,
                cols,
                rows,
                x,
                y,
            }),
            KittyEvent::Delete { id } => self.graphics.delete(id),
        }
    }

    /// 当前应绘制的图像放置列表；渲染器按行映射换算可见位置。
    pub(crate) fn graphics_placements(&self) -> Vec<GraphicsPlacement> {
        self.graphics.placements(self.term.grid().history_size())
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
        self.graphics.clear();
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
