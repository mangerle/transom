use crate::event::TerminalEventProxy;
use crate::osc::{OscEvent, OscScanner};
use crate::renderer::DamageInfo;
use crate::search::SearchState;

/// 视图来不及取走时允许暂存的 OSC 事件上限。
const MAX_PENDING_OSC_EVENTS: usize = 64;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::Cell;
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
    /// 全文检索状态（查询、命中快照与游标），行为见 [`crate::search`]。
    pub(crate) search: SearchState,
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
            search: SearchState::default(),
            last_write: None,
            epoch: 0,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.last_write = Some(Instant::now());
            self.epoch = self.epoch.wrapping_add(1);
            // 先旁路扫 OSC，再把同一份字节交给内核解析器：扫描器不消费输入，
            // 因此畸形序列也只影响通知，不影响屏幕内容。
            self.osc_events.extend(self.osc_scanner.feed(bytes));
            if self.osc_events.len() > MAX_PENDING_OSC_EVENTS {
                // 视图长时间没取走（例如标签被隐藏）时丢最旧的，绝不让队列无界增长。
                let drop = self.osc_events.len() - MAX_PENDING_OSC_EVENTS;
                self.osc_events.drain(..drop);
            }
        }
        self.parser.advance(&mut self.term, bytes);
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
        // 折行重排会让所有缓存行号失去意义。
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::Line;
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
