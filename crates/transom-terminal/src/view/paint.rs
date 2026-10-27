//! 终端画布绘制过程：尺寸同步、损伤跟踪、预计算与 GPUI 提交。

use super::{LayoutMetrics, PreparedTerminal, SEARCH_TICK_PIXELS, TerminalView};
use crate::renderer::{PaintRequest, TerminalRenderer};
use gpui_kit::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use transom_core::TerminalSize;

impl TerminalView {
    /// 构造终端画布元素（包含 prepaint 计算网格与 paint 实际提交）。
    pub(super) fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine_prepaint = self.engine.clone();
        let engine_paint = self.engine.clone();
        let metrics_prepaint = self.metrics.clone();
        let renderer_prepaint = self.renderer.clone();
        let renderer_paint = self.renderer.clone();
        let row_cache_paint = self.row_cache.clone();
        let renderer_dirty = self.renderer_dirty.clone();
        let control = self.control.clone();
        let profile = self.profile.clone();
        let focus_handle = self.focus_handle.clone();
        let entity = cx.entity();
        let ime_text = self.ime_text.clone();
        let ime_marked_range = self.ime_marked_range.clone();
        let cursor_blink_on = self.cursor_blink_on;
        let search_open = self.search_open;
        let block_view = self.block_view;
        let block_style = block_view
            .map(|settings| settings.style)
            .unwrap_or_default();
        let block_layout_shared = self.block_layout.clone();

        canvas(
            move |bounds, window, cx| {
                // 外观变更（脏标记）在此重建：只有 prepaint 阶段持有
                // 窗口文本系统，度量（"M"/"中" 整形探测）必须在这里做。
                let mut renderer_rebuilt = false;
                if renderer_dirty.swap(false, Ordering::AcqRel) {
                    *renderer_prepaint.write() = TerminalRenderer::new(&profile, window, cx);
                    renderer_rebuilt = true;
                }
                let renderer = renderer_prepaint.read();
                let (cols, rows) = renderer.grid_size(bounds);
                let size = TerminalSize::new(cols, rows);
                let mut engine = engine_prepaint.lock();
                let mut resized = false;
                if engine.size() != size {
                    engine.resize(size);
                    resized = true;
                    // 尺寸同步失败会直接导致网格与 PTY 失配（提示符错位），
                    // 因此不再静默忽略，按警告记录并给出排查方向
                    if control.resize(size) {
                        log::debug!(
                            "终端网格尺寸已更新并同步至 PTY，列数: {}，行数: {}",
                            size.cols,
                            size.rows
                        );
                    } else {
                        log::warn!(
                            "PTY 窗口尺寸同步失败（控制通道已断开），画面可能持续错位，请重连会话，列数: {}，行数: {}",
                            size.cols,
                            size.rows
                        );
                    }
                }
                // 检索重扫放在尺寸变更之后：缩放会重排折行、让快照作废，
                // 放在数据流路径里又会把刷屏变成卡顿，所以只在渲染帧里按冷却触发。
                let search = if search_open {
                    engine.refresh_search(std::time::Instant::now());
                    let tick_rows =
                        (f32::from(bounds.size.height) / SEARCH_TICK_PIXELS).max(1.0) as usize;
                    engine.search_paint(tick_rows)
                } else {
                    None
                };
                // 损伤集合必须在持锁的准备阶段取走并重置：它是"上次取走
                // 之后全部变化"的完整窗口。折行重排与外观重建让行号、字体
                // 度量整体失效，无论内核报了什么都提升为全量。
                let mut damage = engine.take_damage();
                if renderer_rebuilt || resized {
                    damage = crate::renderer::DamageInfo::Full;
                }
                // 刻度带只有在真的画了刻度时才登记靶区，否则会白白吃掉一列点击。
                let strip = search
                    .as_ref()
                    .filter(|paint| !paint.ticks.is_empty())
                    .map(|_| TerminalRenderer::search_strip_bounds(bounds));
                let metrics = LayoutMetrics {
                    origin: Point {
                        x: bounds.origin.x + TerminalRenderer::TERMINAL_PADDING_X,
                        y: bounds.origin.y + TerminalRenderer::TERMINAL_PADDING_Y,
                    },
                    cell_width: renderer.cell_width,
                    cell_height: renderer.cell_height,
                    cols: cols as usize,
                    rows: rows as usize,
                    search_strip: strip,
                    bounds: Some(bounds),
                };
                *metrics_prepaint.lock() = metrics;
                let layout = engine
                    .build_block_layout(block_view.as_ref())
                    .map(Arc::new);
                *block_layout_shared.lock() = layout.clone();
                let graphics = engine.graphics_placements();
                PreparedTerminal {
                    search,
                    damage,
                    block_layout: layout,
                    graphics,
                }
            },
            move |bounds, prepared, window, cx| {
                window.handle_input(
                    &focus_handle,
                    ElementInputHandler::new(bounds, entity.clone()),
                    cx,
                );
                let engine = engine_paint.lock();
                let renderer = renderer_paint.read();
                let mut row_cache = row_cache_paint.lock();
                let ime = if ime_text.is_empty() {
                    None
                } else {
                    Some((ime_text.as_str(), ime_marked_range.as_ref()))
                };
                renderer.paint(
                    PaintRequest {
                        bounds,
                        term: engine.term(),
                        ime,
                        blink_visible: cursor_blink_on,
                        search: prepared.search.as_ref(),
                        block_layout: prepared.block_layout.as_deref(),
                        block_style,
                        graphics: &prepared.graphics,
                    },
                    &mut row_cache,
                    &prepared.damage,
                    window,
                    cx,
                );
            },
        )
        .size_full()
    }
}
