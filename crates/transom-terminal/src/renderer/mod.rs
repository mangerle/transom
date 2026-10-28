mod decorations;
mod palette;
mod runs;
mod scan;
mod search;
mod types;

use decorations::BlockBannerPaintParams;
use runs::RowScanInput;
pub(crate) use types::{
    DamageInfo, ImePaintInput, PaintRequest, RowVisual, SELECTION_ALPHA, TextVisual,
};

#[cfg(test)]
mod tests;

use crate::block::VisualSlot;
use crate::highlight::{CompiledHighlight, HighlightSpan};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
pub(crate) use palette::PaletteColors;
use scan::{CJK_PROBE_CHAR, RowScratch, fit_cjk_size, selection_segment};
use std::ops::Range;
use transom_core::{Rgb as PaletteColor, TerminalHighlight, TerminalPalette, TerminalProfile};

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
    /// 静态调色板的 Hsla 预计算表：命名色、前 16 色与 256 色回退值构建时一次算好，
    /// 渲染热路径查表取回，避免同一颜色在每个 cell 上重复做 RGB→HSL 浮点换算。
    pub(crate) palette_colors: PaletteColors,
    /// 关键词自动高亮配置。
    pub(crate) highlight: TerminalHighlight,
    /// 编译后的规则集。渲染器改为长生命周期后只在构建/外观变更时编译一次，
    /// 不再随帧重编译（原先每帧重建时每个关键词都要一次小堆分配）。
    pub(crate) highlight_rules: CompiledHighlight,
    line_height: f32,
    /// 背景不透明度系数：1.0 为完全不透明，按背景透明度设置折算。
    background_alpha: f32,
}

/// 背景透明度（百分比）折算成不透明度系数。
///
/// 百分比先钳到 100 再折算：外部配置里的越界值不该把 alpha 算成负数，
/// 那会让整块背景消失而不是半透明。
fn background_alpha(transparency: u8) -> f32 {
    1.0 - f32::from(transparency.min(100)) / 100.0
}

/// 静态调色板的预计算 Hsla 集合。
///
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
            palette_colors: PaletteColors::build(&profile.palette),
            highlight: profile.highlight.clone(),
            highlight_rules: CompiledHighlight::compile(&profile.highlight),
            line_height: profile.line_height.max(1.0),
            background_alpha: background_alpha(profile.background_transparency),
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

    pub fn paint(
        &self,
        request: PaintRequest<'_>,
        row_cache: &mut Vec<RowVisual>,
        damage: &DamageInfo,
        window: &mut Window,
        cx: &mut App,
    ) {
        let PaintRequest {
            bounds,
            term,
            ime,
            blink_visible,
            search,
            block_layout,
            block_style,
            graphics,
        } = request;
        let grid = term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();
        let colors = term.colors();
        let default_bg = self.resolve(Color::Named(NamedColor::Background), colors);
        // 背景按设置折算透明度：整块默认背景变半透明，透出下层应用背景。
        // 显式设了背景色的单元格仍按原色绘制，不跟着一起洗白。
        //
        // 比较必须用未折算的原色：色块区段里"与默认背景同色"的区段靠这个
        // 比较跳过，交给上面那块半透明底子铺；拿折算后的颜色去比，默认背景
        // 的区段会被当成普通色块再画一遍不透明，整块终端又变实色。
        let default_fill = default_bg.alpha(default_bg.a * self.background_alpha);
        window.paint_quad(fill(bounds, default_fill));

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
        // 折叠模式下光标行按可见行映射定位；落在被折叠区间内时直接隐藏。
        let cursor_row = match block_layout {
            Some(layout) => layout.row_of_line(cursor.line).map(|row| row as i32),
            None => Some(cursor.line.0 + display_offset),
        };
        let cursor_row_raw = cursor_row.unwrap_or(-1);
        let cursor_col = cursor.column.0;
        let cursor_style = term.cursor_style();
        let cursor_color = self.resolve(Color::Named(NamedColor::Cursor), colors);
        let cursor_visible = blink_visible
            && term.mode().contains(TermMode::SHOW_CURSOR)
            && cursor_style.shape != CursorShape::Hidden
            && cursor_row.is_some_and(|row| (0..rows as i32).contains(&row))
            && ime.is_none();
        let block_cursor = cursor_visible && cursor_style.shape == CursorShape::Block;

        // Kitty 图像画在默认底色之上、文本之下：图片不会遮住程序输出的字，
        // 程序显式设置的单元格背景仍然覆盖图片，与主流终端一致。
        for graphic in graphics {
            let row = match block_layout {
                Some(layout) => layout.row_of_line(graphic.line),
                None => {
                    let row = graphic.line.0 + display_offset;
                    (0..rows as i32).contains(&row).then_some(row as usize)
                }
            };
            let Some(row) = row else {
                continue;
            };
            if graphic.col >= cols {
                continue;
            }
            let image_cols = graphic.cols.unwrap_or_else(|| {
                ((graphic.width as f32 / f32::from(self.cell_width)).ceil() as usize).max(1)
            });
            let image_rows = graphic.rows.unwrap_or_else(|| {
                ((graphic.height as f32 / f32::from(self.cell_height)).ceil() as usize).max(1)
            });
            let image_bounds = Bounds {
                origin: Point {
                    x: origin.x + self.cell_width * graphic.col as f32 + px(graphic.x as f32),
                    y: origin.y + self.cell_height * row as f32 + px(graphic.y as f32),
                },
                size: Size {
                    width: self.cell_width * image_cols as f32,
                    height: self.cell_height * image_rows as f32,
                },
            };
            let _ = window.paint_image(
                bounds,
                image_bounds,
                Corners::default(),
                graphic.image.clone(),
                0,
                false,
            );
        }

        // 行缓存与网格行数对齐：resize 后长度不符即整体失效（内核 resize 也会
        // 报告 Full 损伤，两道保险互为冗余）。
        if row_cache.len() != rows {
            row_cache.clear();
            row_cache.resize_with(rows, RowVisual::default);
        }

        // 需要重建的行：内核损伤集合之外，强制加入光标行——闪烁相位翻转
        // 不产生网格损伤，但块光标的字符反色随相位变化。
        let mut damaged_rows: Vec<usize> = match (block_layout.is_some(), damage) {
            // 折叠改变了行映射本身：任何局部损伤都可能移动整屏内容，直接全量重建。
            (true, _) => (0..rows).collect(),
            (false, DamageInfo::Full) => (0..rows).collect(),
            (false, DamageInfo::Rows(list)) => {
                let mut rows_set = Vec::with_capacity(list.len() + 1);
                rows_set.extend_from_slice(list);
                if cursor_visible && (0..rows).contains(&(cursor_row_raw.max(0) as usize)) {
                    rows_set.push(cursor_row_raw as usize);
                }
                rows_set
            }
        };
        damaged_rows.sort_unstable();
        damaged_rows.dedup();

        // 逐行扫描的缓冲在整批重建间复用，避免每行都重新分配字符串与列映射。
        let mut scratch = RowScratch::default();
        let cursor_cell = block_cursor.then_some((cursor_row_raw, cursor_col));
        for row in damaged_rows {
            if row >= rows {
                continue;
            }
            let line = Line(row as i32 - display_offset);
            scratch.fill(
                grid,
                line,
                cols,
                &self.highlight,
                &self.highlight_rules,
                &self.palette,
            );
            // 就地重写而不是整行替换：滚动/刷屏时多数行都在重建，
            // 保留上次的 Vec 容量可避免高吞吐输出下的持续重新分配。
            let mut recycled = std::mem::take(&mut row_cache[row]);
            self.refill_row_visual(
                RowScanInput {
                    grid,
                    line,
                    row,
                    cols,
                    colors,
                    cursor_cell,
                    highlights: &scratch.colors,
                },
                &mut recycled,
            );
            row_cache[row] = recycled;
        }

        for (row, visual) in row_cache.iter().enumerate() {
            let y = origin.y + self.cell_height * row as f32;
            let line = match block_layout {
                Some(layout) => {
                    if let Some(line) = layout.line_at(row) {
                        line
                    } else {
                        if let Some(VisualSlot::Banner { label, failed, .. }) =
                            layout.slots().get(row)
                        {
                            self.paint_block_banner(
                                BlockBannerPaintParams {
                                    row,
                                    y,
                                    origin,
                                    label,
                                    failed: *failed,
                                    style: block_style,
                                    cols,
                                },
                                window,
                                cx,
                            );
                        }
                        continue;
                    }
                }
                None => Line(row as i32 - display_offset),
            };

            for (start, end, color) in &visual.background_runs {
                if *color == default_bg {
                    continue;
                }
                let rect = Bounds {
                    origin: Point {
                        x: origin.x + self.cell_width * *start as f32,
                        y,
                    },
                    size: Size {
                        width: self.cell_width * (end - start) as f32,
                        height: self.cell_height,
                    },
                };
                window.paint_quad(fill(rect, *color));
            }

            if let Some(paint) = search {
                // 检索快照按原始视口行编号；折叠后需要换回该行原本的行号。
                let source_row = match block_layout {
                    Some(_) => (display_offset + line.0).max(0) as usize,
                    None => row,
                };
                self.paint_row_hits(paint.rows.get(source_row), y, origin, window);
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
                let mut color = self.palette_colors.selection;
                color.a = SELECTION_ALPHA;
                window.paint_quad(fill(rect, color));
            }

            if block_cursor && cursor_row_raw == row as i32 && cursor_col < cols {
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

            for run in &visual.text_runs {
                self.paint_run_text(run, origin, row, window, cx);
            }

            if cursor_visible && cursor_row_raw == row as i32 && cursor_col < cols {
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
                ImePaintInput {
                    cursor_row: cursor_row_raw,
                    cursor_col,
                    rows,
                    cols,
                    text,
                    marked_range,
                },
                origin,
                window,
                cx,
            );
        }
    }
}
