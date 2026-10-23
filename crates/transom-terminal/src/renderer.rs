use crate::event::TerminalEventProxy;
use crate::search::SearchPaint;
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
use transom_core::{
    CompiledHighlight, HighlightSpan, Rgb as PaletteColor, TerminalHighlight, TerminalPalette,
    TerminalProfile,
};

const SELECTION_ALPHA: f32 = 0.4;

/// 一次绘制请求的全部输入。
///
/// 收敛成结构体而不是继续平铺参数：绘制需要网格、IME 预编辑、光标相位与检索命中，
/// 参数已超过 5 个，平铺会让调用点难以对齐语义。
pub(crate) struct PaintRequest<'a> {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) term: &'a Term<TerminalEventProxy>,
    pub(crate) ime: Option<(&'a str, Option<&'a Range<usize>>)>,
    pub(crate) blink_visible: bool,
    /// 检索命中绘制数据；`None` 表示当前没有可用快照（未开启或已失效）。
    pub(crate) search: Option<&'a SearchPaint>,
}

pub(crate) struct TerminalRenderer {
    pub font_family: SharedString,
    /// 双格字符（中日韩）的专用字体族；`None` 表示不做双字体路由。
    pub cjk_font_family: Option<SharedString>,
    pub font_size: Pixels,
    /// 双格字体字号：由 [`fit_cjk_size`] 反推到"恰好两格宽"，与 `font_size` 不同。
    pub cjk_font_size: Pixels,
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub(crate) palette: TerminalPalette,
    /// 关键词自动高亮配置。
    pub(crate) highlight: TerminalHighlight,
    /// 每帧编译一次规则集：扫描路径里不再逐行预处理关键词。
    pub(crate) highlight_rules: CompiledHighlight,
    line_height: f32,
}

impl TerminalRenderer {
    pub fn new(profile: &TerminalProfile, window: &mut Window, cx: &mut App) -> Self {
        let font_family = profile
            .font_family
            .clone()
            .map(SharedString::from)
            .unwrap_or_else(|| cx.theme().mono_font_family.clone());
        let cjk_font_family = profile.cjk_font_family.as_ref().map(SharedString::from);
        let mut renderer = Self {
            font_family,
            cjk_font_family,
            font_size: px(profile.font_size.max(6.0)),
            cjk_font_size: px(profile.font_size.max(6.0)),
            cell_width: px(8.0),
            cell_height: px(16.0),
            palette: profile.palette.clone(),
            highlight: profile.highlight.clone(),
            highlight_rules: CompiledHighlight::compile(&profile.highlight),
            line_height: profile.line_height.max(1.0),
        };
        renderer.measure(window);
        renderer
    }

    fn measure(&mut self, window: &mut Window) {
        let run = TextRun {
            len: 1,
            font: self.font_for(false, FontWeight::NORMAL, FontStyle::Normal),
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
        self.measure_cjk(window);
    }

    /// 反推双格字体的字号，让一个汉字正好占两格宽。
    ///
    /// 不做这件事的话，"选了中文字体"仍可能错位：西文等宽字体自带的汉字字面宽度
    /// 普遍不等于 `2 × cell_width`。探针取 `中`——它是 CJK 字体里最稳定的全宽字形。
    fn measure_cjk(&mut self, window: &mut Window) {
        if self.cjk_font_family.is_none() {
            return;
        }
        let run = TextRun {
            len: 1,
            font: self.font_for(true, FontWeight::NORMAL, FontStyle::Normal),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let probe =
            window
                .text_system()
                .shape_line(CJK_PROBE_CHAR.into(), self.font_size, &[run], None);
        self.cjk_font_size = fit_cjk_size(probe.width(), self.cell_width * 2.0, self.font_size);
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

    fn font_for(&self, wide: bool, weight: FontWeight, style: FontStyle) -> Font {
        let cjk = self.uses_cjk_font(wide);
        Font {
            family: if cjk {
                self.cjk_font_family
                    .clone()
                    .unwrap_or_else(|| self.font_family.clone())
            } else {
                self.font_family.clone()
            },
            features: FontFeatures::default(),
            fallbacks: None,
            weight,
            style,
        }
    }

    /// 该字符是否应改走双格字体：只在确实配置了中文字体时才成立。
    fn uses_cjk_font(&self, wide: bool) -> bool {
        wide && self.cjk_font_family.is_some()
    }

    fn font_size_for(&self, wide: bool) -> Pixels {
        if self.uses_cjk_font(wide) {
            self.cjk_font_size
        } else {
            self.font_size
        }
    }

    pub(crate) fn color(&self, color: PaletteColor) -> Hsla {
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

    pub fn paint(&self, request: PaintRequest<'_>, window: &mut Window, cx: &mut App) {
        let PaintRequest {
            bounds,
            term,
            ime,
            blink_visible,
            search,
        } = request;
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
        let cursor_visible = blink_visible
            && term.mode().contains(TermMode::SHOW_CURSOR)
            && cursor_style.shape != CursorShape::Hidden
            && (0..rows as i32).contains(&cursor_row)
            && ime.is_none();
        let block_cursor = cursor_visible && cursor_style.shape == CursorShape::Block;
        // 逐行扫描的缓冲在整帧里复用，避免每行都重新分配字符串与列映射。
        let mut scratch = RowScratch::default();

        for row in 0..rows {
            let line = Line(row as i32 - display_offset);
            let y = origin.y + self.cell_height * row as f32;

            scratch.fill(
                grid,
                line,
                cols,
                &self.highlight,
                &self.highlight_rules,
                &self.palette,
            );
            self.paint_backgrounds(
                origin,
                y,
                RowInput {
                    line,
                    cols,
                    grid,
                    colors,
                    cursor_cell: None,
                    highlights: &scratch.colors,
                },
                default_bg,
                window,
            );

            if let Some(paint) = search {
                self.paint_row_hits(paint.rows.get(row), y, origin, window);
            }

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
                RowInput {
                    line,
                    cols,
                    grid,
                    colors,
                    cursor_cell: block_cursor.then_some((cursor_row, cursor_col)),
                    highlights: &scratch.colors,
                },
                window,
                cx,
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

        if let Some(paint) = search {
            self.paint_search_strip(bounds, paint, window);
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

    fn paint_backgrounds(
        &self,
        origin: Point<Pixels>,
        y: Pixels,
        row: RowInput<'_>,
        default_bg: Hsla,
        window: &mut Window,
    ) {
        let RowInput {
            line,
            cols,
            grid,
            colors,
            ..
        } = row;
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

    fn paint_text_row(
        &self,
        origin: Point<Pixels>,
        row: usize,
        input: RowInput<'_>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let RowInput {
            line,
            cols,
            grid,
            colors,
            cursor_cell,
            highlights,
        } = input;
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
            // 关键词上色只覆盖"程序自己没着色"的字符：已经是红字的 ERROR 再压一层
            // 我们的红只是白做功，还会抢掉程序刻意表达的语义。
            if let Some(override_fg) = highlights.get(col).copied().flatten()
                && cell.fg == Color::Named(NamedColor::Foreground)
            {
                fg = override_fg;
            }
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
                // 网格已经用 WIDE_CHAR 标记替我们判过宽度（含 wcwidth 的全部边界），
                // 直接采信标记，比在渲染侧再维护一份宽度表更可靠。
                wide: cell.flags.contains(Flags::WIDE_CHAR),
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
        let font = self.font_for(
            style.wide,
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
            self.font_size_for(style.wide),
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
            let wide = is_cjk_char(ch);
            let run = TextRun {
                len: ch.len_utf8(),
                font: self.font_for(wide, FontWeight::NORMAL, FontStyle::Normal),
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
                self.font_size_for(wide),
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

/// 单个屏幕行的绘制输入。
///
/// 打包成结构体而不是继续平铺：原先网格、列数、颜色表、光标格已经排到 9 个参数，
/// 再加一份逐列高亮色就会更多，参数顺序也更容易传错。
struct RowInput<'a> {
    line: Line,
    cols: usize,
    grid: &'a alacritty_terminal::grid::Grid<Cell>,
    colors: &'a Colors,
    cursor_cell: Option<(i32, usize)>,
    /// 每个列上该覆盖的前景色；`None` 表示该列不上色。
    highlights: &'a [Option<Hsla>],
}

/// 一行关键词扫描的复用缓冲。
#[derive(Default)]
struct RowScratch {
    /// 去掉宽字符占位格后的行文本。
    text: String,
    /// 每个字符对应的列号。
    columns: Vec<usize>,
    /// 每个字符占用的列宽（宽字符为 2）。
    widths: Vec<usize>,
    /// 命中区间展开到每列的前景色。
    colors: Vec<Option<Hsla>>,
    spans: Vec<HighlightSpan>,
}

impl RowScratch {
    /// 扫描一行，把命中的关键词展开成"每列一个覆盖色"。
    ///
    /// 收配置/规则集/调色板三个引用而不是 `&TerminalRenderer`：后者只能在带窗口的环境里
    /// 构造，那样最容易出错的"字符下标 → 真实列号"换算就没法在单元测试里直接验证。
    fn fill(
        &mut self,
        grid: &alacritty_terminal::grid::Grid<Cell>,
        line: Line,
        cols: usize,
        config: &TerminalHighlight,
        rules: &CompiledHighlight,
        palette: &TerminalPalette,
    ) {
        self.colors.clear();
        self.colors.resize(cols, None);
        if !config.enabled {
            return;
        }
        self.text.clear();
        self.columns.clear();
        self.widths.clear();
        for col in 0..cols {
            let cell = &grid[AlacPoint::new(line, Column(col))];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            self.columns.push(col);
            self.widths.push(if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            });
            self.text.push(cell.c);
        }
        if self.text.is_empty() {
            return;
        }
        self.spans = rules.scan(&self.text, config);
        for span in &self.spans {
            // 命中区间是按"打印字符"编号的，必须换算回真实列号：宽字符占两列，
            // 直接把字符下标当列号会让中文行整体错位；反过来把列号拿去索引字符数组
            // 会把区间画宽。两个索引空间要分开取，不能混用。
            let Some(last_char) = span.end.checked_sub(1) else {
                continue;
            };
            let (Some(&first), Some(&last_col), Some(&width)) = (
                self.columns.get(span.start),
                self.columns.get(last_char),
                self.widths.get(last_char),
            ) else {
                continue;
            };
            let end_col = (last_col + width).min(cols);
            if end_col <= first {
                continue;
            }
            let color = Hsla::from(rgb(palette.ansi[span.kind.ansi_index().min(15)].to_hex()));
            for slot in &mut self.colors[first..end_col] {
                *slot = Some(color);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    /// 是否属于双格字符：决定取主字体还是中文专用字体，必须参与 run 合并判定，
    /// 否则两种字体的字符会被拼进同一次 `shape_line`。
    wide: bool,
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
            && self.style.wide == cell.flags.contains(Flags::WIDE_CHAR)
    }
}

/// 双格字号探针用的字符：`中` 在 CJK 字体里是最稳定的全宽字形。
const CJK_PROBE_CHAR: char = '中';

/// IME 预编辑串里哪些字符按中文专用字体渲染。
///
/// 正文走的是网格的 `WIDE_CHAR` 标记，只有预编辑阶段的字符还没进网格、没有标记可用，
/// 才需要这份范围表兜底：范围取 Unicode 东亚宽度表里判为 Wide 的那批（汉字及扩展、假名、
/// 谚文、注音、中日韩标点与全角形式），半角假名从 U+FF61 起只占一格、刻意排除。
/// 判定偏窄只会让候选字暂时用回退字体渲染，偏宽才会撑破邻格，因此宁可偏窄。
fn is_cjk_char(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115F      // 谚文字母（初声，占两格）
        | 0x2E80..=0x303E    // 中日韩部首补充、康熙部首、假名、注音以外的中日韩符号标点
        | 0x3041..=0x33FF    // 平假名、片假名、注音符号、中日韩兼容符号
        | 0x3400..=0x4DBF    // 汉字扩展 A
        | 0x4E00..=0x9FFF    // 中日韩统一表意文字
        | 0xA000..=0xA4CF    // 彝文
        | 0xAC00..=0xD7A3    // 谚文音节
        | 0xF900..=0xFAFF    // 中日韩兼容表意文字
        | 0xFE30..=0xFE6F    // 小写变体、中日韩兼容形式、全角小符号
        | 0xFF00..=0xFF60    // 全角形式（半角假名从 FF61 起，不在此列）
        | 0xFFE0..=0xFFE6    // 全角货币与空格
        | 0x20000..=0x2FFFD  // 汉字扩展 B 及之后
        | 0x30000..=0x3FFFD
    )
}

/// 把双格字体字号缩放到"一个汉字恰好等于两格宽"。
///
/// 夹在基准字号的 ±50% 内：字体不存在时 gpui 会回退到别的字形、宽度探针可能离谱，
/// 与其把中文拽成蚂蚁字，不如保持接近原字号的结果。
fn fit_cjk_size(probe_width: Pixels, target_width: Pixels, base: Pixels) -> Pixels {
    let natural = f32::from(probe_width);
    let target = f32::from(target_width);
    if natural <= 0.0 || target <= 0.0 {
        return base;
    }
    let scaled = f32::from(base) * (target / natural);
    px(scaled
        .clamp(f32::from(base) * 0.5, f32::from(base) * 1.5)
        .max(1.0))
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

#[cfg(test)]
mod tests {
    use super::{CJK_PROBE_CHAR, RowScratch, fit_cjk_size, is_cjk_char};
    use crate::engine::TerminalEngine;
    use alacritty_terminal::index::Line;
    use gpui_kit::{Hsla, px, rgb};
    use transom_core::{
        CompiledHighlight, TerminalHighlight, TerminalPalette, TerminalProfile, TerminalSize,
    };

    /// 双字体路由的判定边界：走错一侧就会让 Latin 字符被中文字体接管，
    /// 或让全角字符继续用等宽字体而错位。
    #[test]
    fn classifies_double_width_characters() {
        assert!(is_cjk_char('中'));
        assert!(is_cjk_char(CJK_PROBE_CHAR));
        assert!(is_cjk_char('あ'));
        assert!(is_cjk_char('カ'));
        assert!(is_cjk_char('한'));
        assert!(is_cjk_char('　')); // 全角空格
        assert!(is_cjk_char('，'));
        assert!(is_cjk_char('！')); // 全角感叹
        assert!(is_cjk_char('Ａ')); // 全角拉丁
        assert!(is_cjk_char('ㄅ')); // 注音符号按东亚宽度占两格
    }

    #[test]
    fn keeps_narrow_characters_on_the_primary_font() {
        for ch in ['a', 'Z', '0', ' ', '~', 'é', 'ß', 'ф', 'é'] {
            assert!(!is_cjk_char(ch), "{ch} 不应被判定为双格");
        }
        // 半角假名在网格里只占一格，用中文专用字体会撑破邻格。
        assert!(!is_cjk_char('ｱ'));
        assert!(!is_cjk_char('¦'));
        // 组合用零宽字符也不该抢双格字体。
        assert!(!is_cjk_char('\u{0301}'));
    }

    #[test]
    fn fits_cjk_size_to_exactly_two_cells() {
        // 探针 20px 的目标是 16px（两格 = 2 × 8px）→ 字号按 0.8 缩放。
        let fitted = fit_cjk_size(px(20.0), px(16.0), px(14.0));
        assert!((f32::from(fitted) - 11.2).abs() < 0.01);
        // 已经正好两格时不变。
        assert_eq!(fit_cjk_size(px(16.0), px(16.0), px(14.0)), px(14.0));
    }

    /// 字体缺失时 gpui 的回退宽度可能离谱，缩放必须夹在可用范围内。
    #[test]
    fn clamps_degenerate_probe_widths() {
        assert_eq!(fit_cjk_size(px(0.0), px(16.0), px(14.0)), px(14.0));
        assert_eq!(fit_cjk_size(px(1.0), px(16.0), px(14.0)), px(21.0));
        assert_eq!(fit_cjk_size(px(1000.0), px(16.0), px(14.0)), px(7.0));
        assert_eq!(fit_cjk_size(px(-3.0), px(16.0), px(14.0)), px(14.0));
        assert_eq!(fit_cjk_size(px(20.0), px(0.0), px(14.0)), px(14.0));
    }

    /// 用一行真实网格内容填充高亮，返回每列覆盖色的十六进制（`None` 表示该列不上色）。
    ///
    /// 断言比色值而不是比调色板下标：默认调色板里 ansi[1] 与 ansi[9] 同值，
    /// 反查下标会先命中前者，测试会变成在测查找顺序而不是测上色。
    fn highlighted_columns(text: &str, keywords: &[&str], cols: u16) -> Vec<Option<u32>> {
        let (tx, _rx) = flume::bounded(16);
        let profile = TerminalProfile {
            highlight: TerminalHighlight {
                enabled: true,
                keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
            },
            ..TerminalProfile::default()
        };
        let mut engine = TerminalEngine::new(TerminalSize::new(cols, 3), &profile, tx);
        engine.process(text.as_bytes());
        let rules = CompiledHighlight::compile(&profile.highlight);
        let mut scratch = RowScratch::default();
        // 扫描时刻意不看渲染结果，只比对"这列该染成调色板里的哪个 ANSI 槽位"。
        scratch.fill(
            engine.grid(),
            Line(0),
            cols as usize,
            &profile.highlight,
            &rules,
            &profile.palette,
        );
        let ansi = &profile.palette.ansi;
        scratch
            .colors
            .iter()
            .map(|color| {
                color.map(|color| {
                    ansi.iter()
                        .find(|candidate| Hsla::from(rgb(candidate.to_hex())) == color)
                        .expect("覆盖色必须来自调色板")
                        .to_hex()
                })
            })
            .collect()
    }

    /// 关键词必须落在它真正所在的列上：ANSI 9 是错误级别（ERROR/FATAL）的槽位。
    #[test]
    fn highlights_expected_columns() {
        let red = TerminalPalette::default().ansi[9].to_hex();
        let colors = highlighted_columns("ERROR boom here", &[], 20);
        assert_eq!(colors[0], Some(red));
        assert_eq!(colors[4], Some(red));
        assert_eq!(colors[5], None, "关键词后一列不该继续染色");
    }

    /// 宽字符占两列：只按字符下标定列会让中文行的染色整体左移。
    #[test]
    fn wide_characters_shift_columns() {
        // 「中文」占 0..4 四列，ERROR 起于第 4 列。
        let red = TerminalPalette::default().ansi[9].to_hex();
        let colors = highlighted_columns("中文ERROR tail", &[], 20);
        assert_eq!(colors[3], None, "汉字本身不染色");
        assert_eq!(colors[4], Some(red));
        assert_eq!(colors[8], Some(red));
        assert_eq!(colors[9], None);
    }

    #[test]
    fn custom_keyword_columns_use_green_slot() {
        let green = TerminalPalette::default().ansi[10].to_hex();
        let colors = highlighted_columns("a refused b", &["refused"], 20);
        assert_eq!(colors[2], Some(green));
        assert_eq!(colors[8], Some(green), "refused 覆盖到第 8 列");
        assert_eq!(colors[9], None, "词后空格不该染色");
    }

    #[test]
    fn ipv4_and_uuid_slots() {
        let palette = TerminalPalette::default();
        let colors = highlighted_columns("ip 10.0.0.1 ok", &[], 20);
        assert_eq!(colors[3], Some(palette.ansi[14].to_hex()));
        assert_eq!(colors[10], Some(palette.ansi[14].to_hex()));
        assert_eq!(colors[11], None);
        let colors = highlighted_columns("id 6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4b", &[], 40);
        assert_eq!(colors[3], Some(palette.ansi[13].to_hex()));
    }

    /// 行尾空白列与越界列都不能panic：网格列数会随窗口缩放。
    #[test]
    fn empty_and_partial_rows_are_safe() {
        let colors = highlighted_columns("ERROR", &[], 40);
        assert_eq!(colors.len(), 40);
        assert_eq!(colors[6], None, "行尾空白列不上色");
        let colors = highlighted_columns("", &[], 10);
        assert!(colors.iter().all(|color| color.is_none()));
    }

    /// 关掉总开关时零染色，且不做任何扫描工作。
    #[test]
    fn disabled_highlight_colors_nothing() {
        let (tx, _rx) = flume::bounded(16);
        let profile = TerminalProfile {
            highlight: TerminalHighlight {
                enabled: false,
                keywords: vec!["boom".into()],
            },
            ..TerminalProfile::default()
        };
        let mut engine = TerminalEngine::new(TerminalSize::new(20, 3), &profile, tx);
        engine.process(b"ERROR boom");
        let rules = CompiledHighlight::compile(&profile.highlight);
        let mut scratch = RowScratch::default();
        scratch.fill(
            engine.grid(),
            Line(0),
            20,
            &profile.highlight,
            &rules,
            &profile.palette,
        );
        assert!(scratch.colors.iter().all(|color| color.is_none()));
        assert!(scratch.text.is_empty());
    }

    /// 相邻两行复用同一缓冲：上一行的字符与染色不能残留到下一行。
    #[test]
    fn scratch_reuse_does_not_leak_between_rows() {
        let (tx, _rx) = flume::bounded(16);
        let profile = TerminalProfile::default();
        let mut engine = TerminalEngine::new(TerminalSize::new(20, 3), &profile, tx);
        // 必须带 `\r`：单独的 `\n` 在终端里只是下移一行而不复位列，两行文字会错位到同一列起点。
        let stream = "ERROR first\r\nsecond line\r\n";
        engine.process(stream.as_bytes());
        let rules = CompiledHighlight::compile(&profile.highlight);
        let mut scratch = RowScratch::default();
        scratch.fill(
            engine.grid(),
            Line(0),
            20,
            &profile.highlight,
            &rules,
            &profile.palette,
        );
        assert!(scratch.colors[0].is_some(), "第一行有 ERROR，必须染色");
        scratch.fill(
            engine.grid(),
            Line(1),
            20,
            &profile.highlight,
            &rules,
            &profile.palette,
        );
        assert!(
            scratch.colors.iter().all(|color| color.is_none()),
            "第二行没有关键词，必须全空"
        );
        assert_eq!(
            scratch.text.trim_end(),
            "second line",
            "行尾补白不该留下上一行痕迹"
        );
    }
}
