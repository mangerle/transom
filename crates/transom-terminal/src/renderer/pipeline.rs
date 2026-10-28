//! 终端渲染流水线：拆解分阶段 Render Pass（背景、图像、行扫描重绘与覆盖层）。
//!
//! 设计原理（Why）：
//! - 将原本集中在 `paint` 中的 280+ 行混杂逻辑按渲染阶段拆分为独立的流水线 Pass；
//! - 明确各个阶段绘制顺序：背景底色 -> Kitty 图像 -> 网格行（单元格背景、搜索、选区、文字、光标） -> 搜索刻度带 -> IME 覆盖层；
//! - 降低主文件复杂度与嵌套深度，使得各阶段的参数与逻辑边界清晰、易于独立扩展与性能分析。

use super::TerminalRenderer;
use super::decorations::BlockBannerPaintParams;
use super::runs::RowScanInput;
use super::scan::{RowScratch, selection_segment};
use super::types::{DamageInfo, ImePaintInput, PaintRequest, RowVisual, SELECTION_ALPHA};
use crate::block::VisualSlot;
use crate::kitty_graphics::GraphicsPlacement;
use alacritty_terminal::index::Line;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::*;

/// 终端单帧绘制时的计算上下文。
pub(super) struct RenderPassContext<'a> {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) origin: Point<Pixels>,
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) display_offset: i32,
    pub(super) default_bg: Hsla,
    pub(super) selection: Option<SelectionRange>,
    pub(super) cursor_row_raw: i32,
    pub(super) cursor_col: usize,
    pub(super) cursor_color: Hsla,
    pub(super) cursor_shape: CursorShape,
    pub(super) cursor_visible: bool,
    pub(super) block_cursor: bool,
    pub(super) request: PaintRequest<'a>,
}

impl TerminalRenderer {
    /// 第一阶段：绘制终端默认背景底色（含半透明折算）。
    pub(super) fn paint_background_pass(
        &self,
        bounds: Bounds<Pixels>,
        default_bg: Hsla,
        window: &mut Window,
    ) {
        let default_fill = default_bg.alpha(default_bg.a * self.background_alpha);
        window.paint_quad(fill(bounds, default_fill));
    }

    /// 第二阶段：绘制 Kitty 协议图形（画在默认底色之上、文本之下）。
    pub(super) fn paint_graphics_pass(
        &self,
        ctx: &RenderPassContext<'_>,
        graphics: &[GraphicsPlacement],
        window: &mut Window,
    ) {
        for graphic in graphics {
            let row = match ctx.request.block_layout {
                Some(layout) => layout.row_of_line(graphic.line),
                None => {
                    let row = graphic.line.0 + ctx.display_offset;
                    (0..ctx.rows as i32).contains(&row).then_some(row as usize)
                }
            };
            let Some(row) = row else {
                continue;
            };
            if graphic.col >= ctx.cols {
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
                    x: ctx.origin.x + self.cell_width * graphic.col as f32 + px(graphic.x as f32),
                    y: ctx.origin.y + self.cell_height * row as f32 + px(graphic.y as f32),
                },
                size: Size {
                    width: self.cell_width * image_cols as f32,
                    height: self.cell_height * image_rows as f32,
                },
            };
            let _ = window.paint_image(
                ctx.bounds,
                image_bounds,
                Corners::default(),
                graphic.image.clone(),
                0,
                false,
            );
        }
    }

    /// 第三阶段（前置准备）：基于损伤信息重构脏行视觉产物缓存。
    pub(super) fn rebuild_dirty_rows(
        &self,
        ctx: &RenderPassContext<'_>,
        row_cache: &mut Vec<RowVisual>,
        damage: &DamageInfo,
    ) {
        if row_cache.len() != ctx.rows {
            row_cache.clear();
            row_cache.resize_with(ctx.rows, RowVisual::default);
        }

        // 块视图只改变"视口行 -> 网格行"的映射，网格内容本身仍按内核损伤
        // 增量变化；全量只在布局切换（引擎用 Full 报告）或内核要求时发生。
        let mut damaged_rows: Vec<usize> = match damage {
            DamageInfo::Full => (0..ctx.rows).collect(),
            DamageInfo::Rows(list) => {
                let mut rows_set = Vec::with_capacity(list.len() + 1);
                rows_set.extend_from_slice(list);
                if ctx.cursor_visible
                    && (0..ctx.rows).contains(&(ctx.cursor_row_raw.max(0) as usize))
                {
                    rows_set.push(ctx.cursor_row_raw as usize);
                }
                rows_set
            }
        };
        damaged_rows.sort_unstable();
        damaged_rows.dedup();

        let grid = ctx.request.term.grid();
        let colors = ctx.request.term.colors();
        let mut scratch = RowScratch::default();
        let cursor_cell = ctx
            .block_cursor
            .then_some((ctx.cursor_row_raw, ctx.cursor_col));

        for row in damaged_rows {
            if row >= ctx.rows {
                continue;
            }
            // 重建与绘制必须用同一套映射：块视图下视口行对应的是布局里的
            // 网格行，直接按 display_offset 反推会在折叠后画错行。
            let line = match ctx.request.block_layout {
                Some(layout) => match layout.line_at(row) {
                    Some(line) => line,
                    // 横幅与留白没有网格内容，缓存保持默认。
                    None => continue,
                },
                None => Line(row as i32 - ctx.display_offset),
            };
            scratch.fill(
                grid,
                line,
                ctx.cols,
                &self.highlight,
                &self.highlight_rules,
                &self.palette,
            );
            let mut recycled = std::mem::take(&mut row_cache[row]);
            self.refill_row_visual(
                RowScanInput {
                    grid,
                    line,
                    row,
                    cols: ctx.cols,
                    colors,
                    cursor_cell,
                    highlights: &scratch.colors,
                },
                &mut recycled,
            );
            row_cache[row] = recycled;
        }
    }

    /// 第四阶段：逐行渲染网格内容（行背景、折叠横幅、检索匹配、选区、文本 Runs 与光标）。
    pub(super) fn paint_grid_rows_pass(
        &self,
        ctx: &RenderPassContext<'_>,
        row_cache: &[RowVisual],
        window: &mut Window,
        cx: &mut App,
    ) {
        for (row, visual) in row_cache.iter().enumerate() {
            let y = ctx.origin.y + self.cell_height * row as f32;
            let line = match ctx.request.block_layout {
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
                                    origin: ctx.origin,
                                    label,
                                    failed: *failed,
                                    style: ctx.request.block_style,
                                    cols: ctx.cols,
                                },
                                window,
                                cx,
                            );
                        }
                        continue;
                    }
                }
                None => Line(row as i32 - ctx.display_offset),
            };

            // 1. 单元格背景区段
            for (start, end, color) in &visual.background_runs {
                if *color == ctx.default_bg {
                    continue;
                }
                let rect = Bounds {
                    origin: Point {
                        x: ctx.origin.x + self.cell_width * *start as f32,
                        y,
                    },
                    size: Size {
                        width: self.cell_width * (end - start) as f32,
                        height: self.cell_height,
                    },
                };
                window.paint_quad(fill(rect, *color));
            }

            // 2. 检索命中高亮
            if let Some(paint) = ctx.request.search {
                let source_row = match ctx.request.block_layout {
                    Some(_) => (ctx.display_offset + line.0).max(0) as usize,
                    None => row,
                };
                self.paint_row_hits(paint.rows.get(source_row), y, ctx.origin, window);
            }

            // 3. 选区高亮
            if let Some(range) = &ctx.selection
                && let Some((start, end)) = selection_segment(range, line, ctx.cols)
            {
                let rect = Bounds {
                    origin: Point {
                        x: ctx.origin.x + self.cell_width * start as f32,
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

            // 4. 块状光标底色反转高亮
            if ctx.block_cursor && ctx.cursor_row_raw == row as i32 && ctx.cursor_col < ctx.cols {
                let rect = Bounds {
                    origin: Point {
                        x: ctx.origin.x + self.cell_width * ctx.cursor_col as f32,
                        y,
                    },
                    size: Size {
                        width: self.cell_width,
                        height: self.cell_height,
                    },
                };
                window.paint_quad(fill(rect, ctx.cursor_color));
            }

            // 5. 文本 Runs 排版绘制
            for run in &visual.text_runs {
                self.paint_run_text(run, ctx.origin, row, window, cx);
            }

            // 6. 光标造型（非块光标，如下划线、竖线、空心光标）
            if ctx.cursor_visible && ctx.cursor_row_raw == row as i32 && ctx.cursor_col < ctx.cols {
                self.paint_cursor(
                    ctx.origin,
                    y,
                    ctx.cursor_col,
                    ctx.cursor_color,
                    ctx.cursor_shape,
                    window,
                );
            }
        }
    }

    /// 第五阶段：绘制外部覆盖层（搜索刻度带与 IME 候选输入窗口）。
    pub(super) fn paint_overlays_pass(
        &self,
        ctx: &RenderPassContext<'_>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(paint) = ctx.request.search {
            self.paint_search_strip(ctx.bounds, paint, window);
        }

        if let Some((text, marked_range)) = ctx.request.ime.filter(|(text, _)| !text.is_empty()) {
            self.paint_ime(
                ImePaintInput {
                    cursor_row: ctx.cursor_row_raw,
                    cursor_col: ctx.cursor_col,
                    rows: ctx.rows,
                    cols: ctx.cols,
                    text,
                    marked_range,
                },
                ctx.origin,
                window,
                cx,
            );
        }
    }
}
