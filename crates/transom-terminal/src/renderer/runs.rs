//! 文本 run 构造、行视觉产物缓存与整形绘制。

use super::scan::{
    CustomUnderline, RunStyle, TextRunBuilder, UnderlineKind, cell_text, cell_underline_kind,
};
use super::{RowVisual, TerminalRenderer, TextVisual};
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use gpui_kit::*;

/// 一行扫描的全部输入：参数收敛为结构体。
pub(super) struct RowScanInput<'a> {
    pub(super) grid: &'a alacritty_terminal::grid::Grid<Cell>,
    pub(super) line: Line,
    /// 该行的屏幕行号（含 display_offset），块光标反色按它定位。
    pub(super) row: usize,
    pub(super) cols: usize,
    pub(super) colors: &'a Colors,
    pub(super) cursor_cell: Option<(i32, usize)>,
    /// 每列的关键词高亮覆盖色；`None` 表示该列不上色。
    pub(super) highlights: &'a [Option<Hsla>],
}

impl TerminalRenderer {
    /// 把一行网格扫描成绘制产物：单趟列扫描同时聚合背景区段与文本 run。
    ///
    /// 相比改造前"背景一遍、高亮一遍、文本一遍"的三趟扫描，这里每列只解析
    /// 一次颜色；空格并入相邻同风格 run 的规则与旧实现保持一致。
    /// 就地重写调用方传入的 [`RowVisual`]：`clear` 保留两个 Vec 的容量，
    /// 刷屏时（多数行每帧重建）避免持续重新分配。
    pub(super) fn refill_row_visual(&self, input: RowScanInput<'_>, visual: &mut RowVisual) {
        let RowScanInput {
            grid,
            line,
            row,
            cols,
            colors,
            cursor_cell,
            highlights,
        } = input;
        visual.background_runs.clear();
        visual.text_runs.clear();
        let background_runs = &mut visual.background_runs;
        let text_runs = &mut visual.text_runs;
        let mut bg_current: Option<(usize, usize, Hsla)> = None;
        let mut run: Option<TextRunBuilder> = None;

        let flush_text = |run: &mut Option<TextRunBuilder>, text_runs: &mut Vec<TextVisual>| {
            if let Some(builder) = run.take() {
                text_runs.push(TextVisual {
                    text: SharedString::from(builder.text),
                    start_col: builder.start_col,
                    end_col: builder.end_col,
                    style: builder.style,
                });
            }
        };

        for col in 0..cols {
            let cell = &grid[AlacPoint::new(line, Column(col))];
            let (fg, bg) = self.cell_colors(cell, colors);

            if let Some((start, end, color)) = bg_current.as_mut() {
                if *color == bg && *end == col {
                    *end = col + 1;
                } else {
                    background_runs.push((*start, *end, *color));
                    bg_current = Some((col, col + 1, bg));
                }
            } else {
                bg_current = Some((col, col + 1, bg));
            }

            if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if cell.c == ' ' || cell.c == '\0' {
                if let Some(text_run) = run.as_mut()
                    && text_run.style.fg == fg
                    && text_run.style.bold == cell.flags.contains(Flags::BOLD)
                    && text_run.style.italic == cell.flags.contains(Flags::ITALIC)
                    && text_run.style.underline == cell_underline_kind(cell)
                    && text_run.style.wide == cell.flags.contains(Flags::WIDE_CHAR)
                {
                    text_run.text.push(' ');
                    text_run.end_col = col + 1;
                } else {
                    flush_text(&mut run, text_runs);
                }
                continue;
            }

            let mut fg = fg;
            // 关键词上色只覆盖"程序自己没着色"的字符：已经是红字的 ERROR 再压一层
            // 我们的红只是白做功，还会抢掉程序刻意表达的语义。
            if let Some(override_fg) = highlights.get(col).copied().flatten()
                && cell.fg == Color::Named(NamedColor::Foreground)
            {
                fg = override_fg;
            }
            // cursor_cell 携带的是屏幕行坐标（cursor.line.0 + display_offset），
            // 必须与同为屏幕坐标的 row 比较；误用网格坐标 line.0 会在上滚查看
            // 历史时把反色字符画到错误行，与光标块分离。
            if let Some((cursor_row, cursor_col)) = cursor_cell
                && cursor_row == row as i32
                && cursor_col == col
            {
                fg = bg;
            }
            let style = RunStyle {
                fg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell_underline_kind(cell),
                // 网格已经用 WIDE_CHAR 标记替我们判过宽度（含 wcwidth 的全部边界），
                // 直接采信标记，比在渲染侧再维护一份宽度表更可靠。
                wide: cell.flags.contains(Flags::WIDE_CHAR),
            };

            let text = cell_text(cell);
            let single_cell = cell.flags.contains(Flags::WIDE_CHAR)
                || cell.zerowidth().is_some()
                || !cell.c.is_ascii();

            if single_cell {
                flush_text(&mut run, text_runs);
                text_runs.push(TextVisual {
                    text: SharedString::from(text.as_str()),
                    start_col: col,
                    // 双格字符独占两列（含尾随衬格）
                    end_col: col
                        + if cell.flags.contains(Flags::WIDE_CHAR) {
                            2
                        } else {
                            1
                        },
                    style,
                });
                continue;
            }

            match run.as_mut() {
                Some(text_run) if text_run.style == style && text_run.end_col == col => {
                    text_run.text.push(cell.c);
                    text_run.end_col = col + 1;
                }
                _ => {
                    flush_text(&mut run, text_runs);
                    run = Some(TextRunBuilder {
                        text: cell.c.to_string(),
                        start_col: col,
                        end_col: col + 1,
                        style,
                    });
                }
            }
        }
        flush_text(&mut run, text_runs);
        if let Some((start, end, color)) = bg_current {
            background_runs.push((start, end, color));
        }
    }

    /// 提交单个文本 run：文本来自行缓存（SharedString 克隆零分配），
    /// 字形整形依赖 gpui 文本系统的内部缓存按内容命中。
    pub(super) fn paint_run_text(
        &self,
        visual: &TextVisual,
        origin: Point<Pixels>,
        row: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let TextVisual {
            text,
            start_col,
            end_col,
            style,
        } = visual;
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
            underline: match style.underline {
                UnderlineKind::Single | UnderlineKind::Curl => Some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(style.fg),
                    wavy: style.underline == UnderlineKind::Curl,
                }),
                // 双线/点线/虚线文本系统不直接支持，走 paint_run_custom_underline
                UnderlineKind::Double | UnderlineKind::Dotted | UnderlineKind::Dashed => None,
                UnderlineKind::None => None,
            },
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            text.clone(),
            self.font_size_for(style.wide),
            &[text_run],
            None,
        );
        let _ = shaped.paint(
            Point {
                x: origin.x + self.cell_width * *start_col as f32,
                y: origin.y + self.cell_height * row as f32,
            },
            self.cell_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
        if matches!(
            style.underline,
            UnderlineKind::Double | UnderlineKind::Dotted | UnderlineKind::Dashed
        ) {
            self.paint_run_custom_underline(
                &CustomUnderline {
                    kind: style.underline,
                    start_col: *start_col,
                    end_col: *end_col,
                    origin,
                    row,
                    color: style.fg,
                },
                window,
            );
        }
    }
}
