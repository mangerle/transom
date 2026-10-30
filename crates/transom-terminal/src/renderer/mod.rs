mod decorations;
mod palette;
mod pipeline;
mod runs;
mod scan;
mod search;
mod types;

use pipeline::RenderPassContext;
pub(crate) use types::{
    DamageInfo, ImePaintInput, PaintRequest, RowVisual, SELECTION_ALPHA, TextVisual,
};

#[cfg(test)]
mod tests;

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
use scan::{CJK_PROBE_CHAR, fit_cjk_size};
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
        let grid = request.term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();
        let colors = request.term.colors();
        let default_bg = self.resolve(Color::Named(NamedColor::Background), colors);
        let origin = Point {
            x: request.bounds.origin.x + Self::TERMINAL_PADDING_X,
            y: request.bounds.origin.y + Self::TERMINAL_PADDING_Y,
        };
        let display_offset = grid.display_offset() as i32;
        let selection = request
            .term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(request.term));
        let vi_mode = request.term.mode().contains(TermMode::VI);
        // 复制模式的光标由内核单独持有：它可以在历史缓冲里移动，而真实光标
        // 永远停在提示符处；不切过来就看不到键盘移动的落点。
        let cursor = if vi_mode {
            request.term.vi_mode_cursor.point
        } else {
            grid.cursor.point
        };
        let cursor_row = match request.block_layout {
            Some(layout) => layout.row_of_line(cursor.line).map(|row| row as i32),
            None => Some(cursor.line.0 + display_offset),
        };
        let cursor_row_raw = cursor_row.unwrap_or(-1);
        let cursor_col = cursor.column.0;
        let cursor_style = request.term.cursor_style();
        let cursor_color = self.resolve(Color::Named(NamedColor::Cursor), colors);
        // 复制模式光标常亮：TUI 常常隐藏真实光标或让它闪烁，而浏览历史时
        // 用户需要随时确认"我当前停在哪一格"。
        let cursor_visible = (vi_mode
            || (request.blink_visible && request.term.mode().contains(TermMode::SHOW_CURSOR)))
            && cursor_style.shape != CursorShape::Hidden
            && cursor_row.is_some_and(|row| (0..rows as i32).contains(&row))
            && request.ime.is_none();
        let block_cursor = cursor_visible && cursor_style.shape == CursorShape::Block;
        let graphics = request.graphics;
        let bounds = request.bounds;

        let ctx = RenderPassContext {
            bounds,
            origin,
            rows,
            cols,
            display_offset,
            default_bg,
            selection,
            cursor_row_raw,
            cursor_col,
            cursor_color,
            cursor_shape: cursor_style.shape,
            cursor_visible,
            block_cursor,
            request,
        };

        // 渲染阶段流水线：
        // Pass 1: 终端底色与半透明填充
        self.paint_background_pass(bounds, default_bg, window);
        // Pass 2: Kitty 协议图形绘制
        self.paint_graphics_pass(&ctx, graphics, window);
        // Pass 3: 脏行视觉扫描缓存重构
        self.rebuild_dirty_rows(&ctx, row_cache, damage);
        // Pass 4: 网格行 Runs、选区与光标绘制
        self.paint_grid_rows_pass(&ctx, row_cache, window, cx);
        // Pass 5: 覆盖层绘制（搜索刻度带与输入法浮层）
        self.paint_overlays_pass(&ctx, window, cx);
    }
}
