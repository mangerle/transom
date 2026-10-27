//! 终端装饰层绘制：自绘下划线（双线/点线/虚线）、光标造型与 IME 预编辑。

use super::scan::{
    CustomUnderline, RunStyle, UnderlineKind, is_cjk_char, utf16_range_to_char_range,
};
use super::{ImePaintInput, SELECTION_ALPHA, TerminalRenderer, TextVisual};
use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::*;
use transom_core::BlockViewStyle;

impl TerminalRenderer {
    /// 自绘文本系统不支持下划线的种类（双线/点线/虚线），按单元格几何绘制。
    ///
    /// 设计原理（Why）：gpui 的 UnderlineStyle 只有实线与波浪线两种；图案以
    /// 格为周期与终端网格对齐，不随字体度量漂移；颜色跟随 run 前景色，
    /// 与实线下划线的取色一致。
    pub(super) fn paint_run_custom_underline(&self, spec: &CustomUnderline, window: &mut Window) {
        let CustomUnderline {
            kind,
            start_col,
            end_col,
            origin,
            row,
            color,
        } = *spec;
        let cell_width = f32::from(self.cell_width);
        let cell_height = f32::from(self.cell_height);
        let base_x = f32::from(origin.x) + cell_width * start_col as f32;
        let base_y = f32::from(origin.y) + cell_height * row as f32;
        let run_width = cell_width * (end_col.saturating_sub(start_col)) as f32;
        let paint_segment = |x: f32, y: f32, w: f32, window: &mut Window| {
            if w > 0.0 {
                window.paint_quad(fill(
                    Bounds {
                        origin: Point { x: px(x), y: px(y) },
                        size: Size {
                            width: px(w),
                            height: px(1.0),
                        },
                    },
                    color,
                ));
            }
        };
        match kind {
            UnderlineKind::Double => {
                let y1 = base_y + cell_height * 0.84;
                let y2 = base_y + cell_height * 0.92;
                paint_segment(base_x, y1, run_width, window);
                paint_segment(base_x, y2, run_width, window);
            }
            UnderlineKind::Dotted => {
                let y = base_y + cell_height * 0.88;
                let dot = (cell_width * 0.22).max(1.0);
                for offset in 0..end_col.saturating_sub(start_col) {
                    let x = base_x + cell_width * offset as f32 + cell_width * 0.1;
                    paint_segment(x, y, dot, window);
                }
            }
            UnderlineKind::Dashed => {
                let y = base_y + cell_height * 0.88;
                for offset in 0..end_col.saturating_sub(start_col) {
                    let x = base_x + cell_width * offset as f32;
                    paint_segment(x + cell_width * 0.05, y, cell_width * 0.35, window);
                    paint_segment(x + cell_width * 0.6, y, cell_width * 0.35, window);
                }
            }
            _ => {}
        }
    }

    /// 绘制不同造型的终端光标（竖线、下划线、空心块）。
    pub(crate) fn paint_cursor(
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

    /// 绘制 IME 预编辑输入覆盖层。
    pub(super) fn paint_ime(
        &self,
        input: ImePaintInput<'_>,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let ImePaintInput {
            cursor_row,
            cursor_col,
            rows,
            cols,
            text,
            marked_range,
        } = input;
        if cursor_row < 0 || cursor_row as usize >= rows || cursor_col >= cols {
            return;
        }
        let y = origin.y + self.cell_height * cursor_row as f32;
        let mut highlight = self.palette_colors.selection;
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
                color: self.palette_colors.foreground,
                background_color: None,
                underline: underline.then_some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(self.palette_colors.foreground),
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

    /// 绘制折叠横幅：柔和底色、失败态左侧状态线与一行摘要文本。
    ///
    /// 横幅只占一个网格行，不创建 GPUI 子元素：终端是自绘 canvas，
    /// 保持"所有内容都在同一绘制管线里"才能让折叠/展开只有一次重绘。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_block_banner(
        &self,
        row: usize,
        y: Pixels,
        origin: Point<Pixels>,
        label: &str,
        failed: bool,
        style: BlockViewStyle,
        cols: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let status_color = if failed {
            self.palette_colors.ansi[1]
        } else {
            self.palette_colors.dim_foreground
        };
        let mut background = if failed {
            self.palette_colors.ansi[1]
        } else {
            self.palette_colors.foreground
        };
        background.a = if failed { 0.14 } else { 0.07 };
        let rect = Bounds {
            origin: Point { x: origin.x, y },
            size: Size {
                width: self.cell_width * cols as f32,
                height: self.cell_height,
            },
        };
        window.paint_quad(fill(rect, background));
        if style == BlockViewStyle::Minimal {
            window.paint_quad(fill(
                Bounds {
                    origin: Point { x: origin.x, y },
                    size: Size {
                        width: px(2.0),
                        height: self.cell_height,
                    },
                },
                status_color,
            ));
        }
        let visual = TextVisual {
            text: SharedString::from(label.to_string()),
            start_col: 2,
            end_col: 2 + label.chars().count(),
            style: RunStyle {
                fg: status_color,
                bold: false,
                italic: false,
                underline: UnderlineKind::None,
                wide: false,
            },
        };
        self.paint_run_text(&visual, origin, row, window, cx);
    }
}
