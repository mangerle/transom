use crate::event::TerminalEventProxy;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use transom_core::{TerminalProfile, TerminalSize};

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
    term: Term<TerminalEventProxy>,
    parser: Processor,
    size: TerminalSize,
}

impl TerminalEngine {
    pub fn new(
        size: TerminalSize,
        profile: &TerminalProfile,
        event_tx: flume::Sender<alacritty_terminal::event::Event>,
    ) -> Self {
        let config = Config {
            scrolling_history: profile.scrollback,
            ..Default::default()
        };
        let dimensions = TermDimensions::new(size.cols as usize, size.rows as usize);
        let term = Term::new(config, &dimensions, TerminalEventProxy::new(event_tx));
        Self {
            term,
            parser: Processor::new(),
            size,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, size: TerminalSize) {
        let width = size.cols.max(1) as usize;
        let height = size.rows.max(1) as usize;
        if width == self.size.cols as usize && height == self.size.rows as usize {
            return;
        }
        self.size = TerminalSize::new(width as u16, height as u16);
        self.term.resize(TermDimensions::new(width, height));
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

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
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
}
