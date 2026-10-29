//! 终端渲染器的检索叠加层：命中底色、当前命中强调与右侧刻度带。
//!
//! 与 `renderer` 拆开是因为这一层只在搜索栏展开的一帧里存在，绘制目标（网格列区间
//! 而不是字符字形）和主渲染通路完全不同；放在一起会让主通路文件持续被可选叠加层稀释。

use super::TerminalRenderer;
use crate::search::{RowPaint, SearchPaint};
use gpui_kit::*;

/// 普通检索命中的着色强度：要让字形仍然可读。
const SEARCH_HIT_ALPHA: f32 = 0.35;
/// 当前命中的着色强度，明显高于普通命中以便定位。
const SEARCH_CURRENT_ALPHA: f32 = 0.7;
/// 检索刻度带宽度。
const SEARCH_STRIP_WIDTH: Pixels = px(8.0);
/// 刻度带与内容右边缘的间距。
const SEARCH_STRIP_MARGIN: Pixels = px(2.0);
/// 刻度条两侧各留出的空隙，保证与终端正文之间有可见分界。
const SEARCH_TICK_INSET: Pixels = px(2.0);

impl TerminalRenderer {
    /// 检索刻度带在给定绘制区域内的矩形。
    ///
    /// 绘制与鼠标命中测试必须共用这一套几何，否则点刻度会落到旁边的字符上。
    pub fn search_strip_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: Point {
                x: bounds.origin.x + bounds.size.width - SEARCH_STRIP_WIDTH - SEARCH_STRIP_MARGIN,
                y: bounds.origin.y,
            },
            size: Size {
                width: SEARCH_STRIP_WIDTH,
                height: bounds.size.height,
            },
        }
    }

    /// 绘制一个屏幕行内的检索命中：普通命中先铺，当前命中覆盖其上。
    pub(crate) fn paint_row_hits(
        &self,
        row: Option<&RowPaint>,
        y: Pixels,
        origin: Point<Pixels>,
        window: &mut Window,
    ) {
        let Some(row) = row else {
            return;
        };
        for (start, end) in &row.hits {
            self.paint_hit_span(*start, *end, y, origin, SEARCH_HIT_ALPHA, window);
        }
        if let Some((start, end)) = row.current {
            self.paint_hit_span(start, end, y, origin, SEARCH_CURRENT_ALPHA, window);
        }
    }

    fn paint_hit_span(
        &self,
        start: usize,
        end: usize,
        y: Pixels,
        origin: Point<Pixels>,
        alpha: f32,
        window: &mut Window,
    ) {
        let mut color = self.color(self.palette.cursor);
        color.a = alpha;
        let width = self.cell_width * end.saturating_sub(start).max(1) as f32;
        window.paint_quad(fill(
            Bounds {
                origin: Point {
                    x: origin.x + self.cell_width * start as f32,
                    y,
                },
                size: Size {
                    width,
                    height: self.cell_height,
                },
            },
            color,
        ));
    }

    /// 绘制右侧刻度带：把整个缓冲区的标记分布压缩到一屏高度内，并提供点击跳转的靶区。
    ///
    /// 一条刻度带最多同时承载两类标记：检索命中用前景强调色、失败命令用红色。
    /// 分色是必要的——两类标记的跳转目标不同（命中行 vs 命令提示符行），
    /// 用户在毫秒级的点击里没时间读文字说明，颜色就是他判断"我要点哪个"的唯一依据。
    pub(crate) fn paint_search_strip(
        &self,
        bounds: Bounds<Pixels>,
        paint: Option<&SearchPaint>,
        failures: &[f32],
        window: &mut Window,
    ) {
        let hits = paint.map(|paint| paint.ticks.as_slice()).unwrap_or(&[]);
        if hits.is_empty() && failures.is_empty() {
            return;
        }
        let strip = Self::search_strip_bounds(bounds);
        let mut track = self.color(self.palette.foreground);
        track.a *= 0.1;
        window.paint_quad(fill(strip, track));

        // 刻度按标记条数均分高度并夹到最小 2px，标记很多时仍能看清分布而不是糊成一条线。
        let height = (strip.size.height / (hits.len() + failures.len()).max(1) as f32).max(px(2.0));
        let width = (strip.size.width - SEARCH_TICK_INSET * 2.0).max(px(1.0));
        for (fractions, color) in [
            (hits, {
                let mut color = self.color(self.palette.cursor);
                color.a = 0.85;
                color
            }),
            (failures, {
                let mut color = self.palette_colors.ansi[1];
                color.a = 0.9;
                color
            }),
        ] {
            for fraction in fractions {
                let y = strip.origin.y + strip.size.height * *fraction - height / 2.0;
                window.paint_quad(fill(
                    Bounds {
                        origin: Point {
                            x: strip.origin.x + SEARCH_TICK_INSET,
                            y,
                        },
                        size: Size { width, height },
                    },
                    color,
                ));
            }
        }
    }
}
