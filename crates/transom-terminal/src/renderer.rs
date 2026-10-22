use crate::event::TerminalEventProxy;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use std::ops::Range;
use transom_core::{Rgb as PaletteColor, TerminalPalette, TerminalProfile};

const SELECTION_ALPHA: f32 = 0.4;

pub(crate) struct TerminalRenderer {
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    palette: TerminalPalette,
    line_height: f32,
}

impl TerminalRenderer {
    pub fn new(profile: &TerminalProfile, window: &mut Window, cx: &mut App) -> Self {
        let font_family = profile
            .font_family
            .clone()
            .map(SharedString::from)
            .unwrap_or_else(|| cx.theme().mono_font_family.clone());
        let mut renderer = Self {
            font_family,
            font_size: px(profile.font_size.max(6.0)),
            cell_width: px(8.0),
            cell_height: px(16.0),
            palette: profile.palette.clone(),
            line_height: profile.line_height.max(1.0),
        };
        renderer.measure(window);
        renderer
    }

    fn measure(&mut self, window: &mut Window) {
        let run = TextRun {
            len: 1,
            font: self.font(FontWeight::NORMAL, FontStyle::Normal),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line("M".into(), self.font_size, &[run], None);
        self.cell_width = shaped.width().max(px(1.0));
        self.cell_height = ((shaped.ascent + shaped.descent) * self.line_height).max(px(1.0));
    }

    pub const TERMINAL_PADDING_X: Pixels = px(8.0);
    pub const TERMINAL_PADDING_Y: Pixels = px(4.0);

    pub fn grid_size(&self, bounds: Bounds<Pixels>) -> (u16, u16) {
        let width = f32::from(bounds.size.width) - f32::from(Self::TERMINAL_PADDING_X) * 2.0;
        let height = f32::from(bounds.size.height) - f32::from(Self::TERMINAL_PADDING_Y) * 2.0;
        let cols = (width.max(1.0) / f32::from(self.cell_width))
            .floor()
            .max(1.0)
            .min(u16::MAX as f32) as u16;
        let rows = (height.max(1.0) / f32::from(self.cell_height))
            .floor()
            .max(1.0)
            .min(u16::MAX as f32) as u16;
        (cols, rows)
    }

    fn font(&self, weight: FontWeight, style: FontStyle) -> Font {
        Font {
            family: self.font_family.clone(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight,
            style,
        }
    }

    fn color(&self, color: PaletteColor) -> Hsla {
        Hsla::from(rgb(color.to_hex()))
    }

    fn resolve(&self, color: Color, colors: &Colors) -> Hsla {
        match color {
            Color::Spec(rgb) => self.color(PaletteColor::new(rgb.r, rgb.g, rgb.b)),
            Color::Indexed(index) => self.resolve_indexed(index as usize, colors),
            Color::Named(named) => self.resolve_named(named, colors),
        }
    }

    fn resolve_named(&self, named: NamedColor, colors: &Colors) -> Hsla {
        if let Some(rgb) = colors[named] {
            return self.color(PaletteColor::new(rgb.r, rgb.g, rgb.b));
        }
        match named {
            NamedColor::Black
            | NamedColor::Red
            | NamedColor::Green
            | NamedColor::Yellow
            | NamedColor::Blue
            | NamedColor::Magenta
            | NamedColor::Cyan
            | NamedColor::White
            | NamedColor::BrightBlack
            | NamedColor::BrightRed
            | NamedColor::BrightGreen
            | NamedColor::BrightYellow
            | NamedColor::BrightBlue
            | NamedColor::BrightMagenta
            | NamedColor::BrightCyan
            | NamedColor::BrightWhite => self.color(self.palette.ansi[named as usize]),
            NamedColor::Foreground | NamedColor::BrightForeground => {
                self.color(self.palette.foreground)
            }
            NamedColor::Background => self.color(self.palette.background),
            NamedColor::Cursor => self.color(self.palette.cursor),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => {
                let index = named as usize - NamedColor::DimBlack as usize;
                let mut color = self.color(self.palette.ansi[index]);
                color.a *= 0.7;
                color
            }
            NamedColor::DimForeground => {
                let mut color = self.color(self.palette.foreground);
                color.a *= 0.7;
                color
            }
        }
    }

    fn resolve_indexed(&self, index: usize, colors: &Colors) -> Hsla {
        if let Some(rgb) = colors[index] {
            return self.color(PaletteColor::new(rgb.r, rgb.g, rgb.b));
        }
        if index < 16 {
            return self.color(self.palette.ansi[index]);
        }
        if index < 232 {
            let index = index - 16;
            let r = (index / 36) as u8;
            let g = ((index % 36) / 6) as u8;
            let b = (index % 6) as u8;
            let channel = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
            return self.color(PaletteColor::new(channel(r), channel(g), channel(b)));
        }
        let level = 8 + (index - 232) as u8 * 10;
        self.color(PaletteColor::new(level, level, level))
    }

    fn cell_colors(&self, cell: &Cell, colors: &Colors) -> (Hsla, Hsla) {
        let mut fg = self.resolve(cell.fg, colors);
        let mut bg = self.resolve(cell.bg, colors);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg.a *= 0.7;
        }
        (fg, bg)
    }

    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        term: &Term<TerminalEventProxy>,
        window: &mut Window,
        cx: &mut App,
        ime: Option<(&str, Option<&Range<usize>>)>,
    ) {
        let grid = term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();
        let colors = term.colors();
        let default_bg = self.resolve(Color::Named(NamedColor::Background), colors);
        window.paint_quad(fill(bounds, default_bg));

        let origin = Point {
            x: bounds.origin.x + Self::TERMINAL_PADDING_X,
            y: bounds.origin.y + Self::TERMINAL_PADDING_Y,
        };
        let display_offset = grid.display_offset() as i32;
        let selection = term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(term));
        let cursor = grid.cursor.point;
        let cursor_row = cursor.line.0 + display_offset;
        let cursor_col = cursor.column.0;
        let cursor_style = term.cursor_style();
        let cursor_color = self.resolve(Color::Named(NamedColor::Cursor), colors);
        let cursor_visible = term.mode().contains(TermMode::SHOW_CURSOR)
            && cursor_style.shape != CursorShape::Hidden
            && (0..rows as i32).contains(&cursor_row)
            && ime.is_none();
        let block_cursor = cursor_visible && cursor_style.shape == CursorShape::Block;

        for row in 0..rows {
            let line = Line(row as i32 - display_offset);
            let y = origin.y + self.cell_height * row as f32;

            self.paint_backgrounds(origin, y, line, cols, grid, colors, default_bg, window);

            if let Some(range) = &selection
                && let Some((start, end)) = selection_segment(range, line, cols)
            {
                let rect = Bounds {
                    origin: Point {
                        x: origin.x + self.cell_width * start as f32,
                        y,
                    },
                    size: Size {
                        width: self.cell_width * (end - start) as f32,
                        height: self.cell_height,
                    },
                };
                let mut color = self.color(self.palette.selection);
                color.a = SELECTION_ALPHA;
                window.paint_quad(fill(rect, color));
            }

            if block_cursor && cursor_row == row as i32 && cursor_col < cols {
                let rect = Bounds {
                    origin: Point {
                        x: origin.x + self.cell_width * cursor_col as f32,
                        y,
                    },
                    size: Size {
                        width: self.cell_width,
                        height: self.cell_height,
                    },
                };
                window.paint_quad(fill(rect, cursor_color));
            }

            self.paint_text_row(
                origin,
                row,
                line,
                cols,
                grid,
                colors,
                window,
                cx,
                block_cursor.then_some((cursor_row, cursor_col)),
            );

            if cursor_visible && cursor_row == row as i32 && cursor_col < cols {
                self.paint_cursor(
                    origin,
                    y,
                    cursor_col,
                    cursor_color,
                    cursor_style.shape,
                    window,
                );
            }
        }

        if let Some((text, marked_range)) = ime.filter(|(text, _)| !text.is_empty()) {
            self.paint_ime(
                origin,
                cursor_row,
                cursor_col,
                rows,
                cols,
                text,
                marked_range,
                window,
                cx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_backgrounds(
        &self,
        origin: Point<Pixels>,
        y: Pixels,
        line: Line,
        cols: usize,
        grid: &alacritty_terminal::grid::Grid<Cell>,
        colors: &Colors,
        default_bg: Hsla,
        window: &mut Window,
    ) {
        let mut current: Option<(usize, usize, Hsla)> = None;
        let mut runs = Vec::new();
        for col in 0..cols {
            let cell = &grid[AlacPoint::new(line, Column(col))];
            let (_, bg) = self.cell_colors(cell, colors);
            if let Some((start, end, color)) = &mut current {
                if *color == bg && *end == col {
                    *end = col + 1;
                    continue;
                }
                runs.push((*start, *end, *color));
            }
            current = Some((col, col + 1, bg));
        }
        if let Some(run) = current {
            runs.push(run);
        }

        for (start, end, color) in runs {
            if color == default_bg {
                continue;
            }
            let rect = Bounds {
                origin: Point {
                    x: origin.x + self.cell_width * start as f32,
                    y,
                },
                size: Size {
                    width: self.cell_width * (end - start) as f32,
                    height: self.cell_height,
                },
            };
            window.paint_quad(fill(rect, color));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_text_row(
        &self,
        origin: Point<Pixels>,
        row: usize,
        line: Line,
        cols: usize,
        grid: &alacritty_terminal::grid::Grid<Cell>,
        colors: &Colors,
        window: &mut Window,
        cx: &mut App,
        cursor_cell: Option<(i32, usize)>,
    ) {
        let mut run: Option<TextRunBuilder> = None;
        for col in 0..cols {
            let cell = &grid[AlacPoint::new(line, Column(col))];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if cell.c == ' ' || cell.c == '\0' {
                if let Some(text_run) = run.as_mut()
                    && text_run.style_matches(cell, self, colors)
                {
                    text_run.text.push(' ');
                    text_run.end_col = col + 1;
                    continue;
                }
                self.flush_run(run.take(), origin, row, window, cx);
                continue;
            }

            let (mut fg, bg) = self.cell_colors(cell, colors);
            if let Some((cursor_row, cursor_col)) = cursor_cell
                && cursor_row == line.0
                && cursor_col == col
            {
                fg = bg;
            }
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let underline = cell.flags.intersects(Flags::ALL_UNDERLINES);
            let style = RunStyle {
                fg,
                bold,
                italic,
                underline,
            };

            let text = cell_text(cell);
            let single_cell = cell.flags.contains(Flags::WIDE_CHAR)
                || cell.zerowidth().is_some()
                || !cell.c.is_ascii();

            if single_cell {
                self.flush_run(run.take(), origin, row, window, cx);
                self.paint_run_text(&text, col, style, origin, row, window, cx);
                continue;
            }

            match run.as_mut() {
                Some(text_run) if text_run.style == style && text_run.end_col == col => {
                    text_run.text.push(cell.c);
                    text_run.end_col = col + 1;
                }
                _ => {
                    self.flush_run(run.take(), origin, row, window, cx);
                    run = Some(TextRunBuilder {
                        text: cell.c.to_string(),
                        start_col: col,
                        end_col: col + 1,
                        style,
                    });
                }
            }
        }
        self.flush_run(run, origin, row, window, cx);
    }

    fn flush_run(
        &self,
        run: Option<TextRunBuilder>,
        origin: Point<Pixels>,
        row: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(run) = run {
            self.paint_run_text(&run.text, run.start_col, run.style, origin, row, window, cx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_run_text(
        &self,
        text: &str,
        start_col: usize,
        style: RunStyle,
        origin: Point<Pixels>,
        row: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let font = self.font(
            if style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
        );
        let text_run = TextRun {
            len: text.len(),
            font,
            color: style.fg,
            background_color: None,
            underline: style.underline.then_some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(style.fg),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            self.font_size,
            &[text_run],
            None,
        );
        let _ = shaped.paint(
            Point {
                x: origin.x + self.cell_width * start_col as f32,
                y: origin.y + self.cell_height * row as f32,
            },
            self.cell_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    fn paint_cursor(
        &self,
        origin: Point<Pixels>,
        y: Pixels,
        col: usize,
        color: Hsla,
        shape: CursorShape,
        window: &mut Window,
    ) {
        let x = origin.x + self.cell_width * col as f32;
        match shape {
            CursorShape::Block | CursorShape::Hidden => {}
            CursorShape::Beam => {
                window.paint_quad(fill(
                    Bounds {
                        origin: Point { x, y },
                        size: Size {
                            width: px(2.0),
                            height: self.cell_height,
                        },
                    },
                    color,
                ));
            }
            CursorShape::Underline => {
                window.paint_quad(fill(
                    Bounds {
                        origin: Point {
                            x,
                            y: y + self.cell_height - px(2.0),
                        },
                        size: Size {
                            width: self.cell_width,
                            height: px(2.0),
                        },
                    },
                    color,
                ));
            }
            CursorShape::HollowBlock => {
                let thickness = px(1.0);
                for bounds in [
                    Bounds {
                        origin: Point { x, y },
                        size: Size {
                            width: self.cell_width,
                            height: thickness,
                        },
                    },
                    Bounds {
                        origin: Point {
                            x,
                            y: y + self.cell_height - thickness,
                        },
                        size: Size {
                            width: self.cell_width,
                            height: thickness,
                        },
                    },
                    Bounds {
                        origin: Point { x, y },
                        size: Size {
                            width: thickness,
                            height: self.cell_height,
                        },
                    },
                    Bounds {
                        origin: Point {
                            x: x + self.cell_width - thickness,
                            y,
                        },
                        size: Size {
                            width: thickness,
                            height: self.cell_height,
                        },
                    },
                ] {
                    window.paint_quad(fill(bounds, color));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_ime(
        &self,
        origin: Point<Pixels>,
        cursor_row: i32,
        cursor_col: usize,
        rows: usize,
        cols: usize,
        text: &str,
        marked_range: Option<&Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if cursor_row < 0 || cursor_row as usize >= rows || cursor_col >= cols {
            return;
        }
        let y = origin.y + self.cell_height * cursor_row as f32;
        let mut highlight = self.color(self.palette.selection);
        highlight.a = SELECTION_ALPHA;
        let marked_chars = marked_range.and_then(|range| utf16_range_to_char_range(text, range));

        for (index, ch) in text.chars().enumerate() {
            let x = origin.x + self.cell_width * (cursor_col + index) as f32;
            window.paint_quad(fill(
                Bounds {
                    origin: Point { x, y },
                    size: Size {
                        width: self.cell_width,
                        height: self.cell_height,
                    },
                },
                highlight,
            ));

            let underline = marked_chars
                .as_ref()
                .is_some_and(|range| range.contains(&index));
            let run = TextRun {
                len: ch.len_utf8(),
                font: self.font(FontWeight::NORMAL, FontStyle::Normal),
                color: self.color(self.palette.foreground),
                background_color: None,
                underline: underline.then_some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(self.color(self.palette.foreground)),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(ch.to_string()),
                self.font_size,
                &[run],
                None,
            );
            let _ = shaped.paint(
                Point { x, y },
                self.cell_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
}

struct TextRunBuilder {
    text: String,
    start_col: usize,
    end_col: usize,
    style: RunStyle,
}

impl TextRunBuilder {
    fn style_matches(&self, cell: &Cell, renderer: &TerminalRenderer, colors: &Colors) -> bool {
        let (fg, _) = renderer.cell_colors(cell, colors);
        self.style.fg == fg
            && self.style.bold == cell.flags.contains(Flags::BOLD)
            && self.style.italic == cell.flags.contains(Flags::ITALIC)
            && self.style.underline == cell.flags.intersects(Flags::ALL_UNDERLINES)
    }
}

fn cell_text(cell: &Cell) -> String {
    let mut text = String::new();
    text.push(cell.c);
    if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth);
    }
    text
}

fn selection_segment(
    selection: &SelectionRange,
    line: Line,
    cols: usize,
) -> Option<(usize, usize)> {
    if line < selection.start.line || line > selection.end.line {
        return None;
    }
    let last_col = cols.saturating_sub(1);
    let start = if selection.is_block || line == selection.start.line {
        selection.start.column.0.min(last_col)
    } else {
        0
    };
    let end_inclusive = if selection.is_block || line == selection.end.line {
        selection.end.column.0.min(last_col)
    } else {
        last_col
    };
    (start <= end_inclusive).then_some((start, end_inclusive + 1))
}

fn utf16_range_to_char_range(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let mut utf16_offset = 0;
    let mut start = None;
    let mut end = None;
    for (char_index, ch) in text.chars().enumerate() {
        if utf16_offset == range.start {
            start = Some(char_index);
        }
        if utf16_offset == range.end {
            end = Some(char_index);
            break;
        }
        utf16_offset += ch.len_utf16();
    }
    let char_count = text.chars().count();
    if utf16_offset == range.start {
        start = Some(char_count);
    }
    if utf16_offset == range.end {
        end = Some(char_count);
    }
    match (start, end) {
        (Some(start), Some(end)) if start <= end => Some(start..end),
        _ => None,
    }
}
