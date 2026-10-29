//! 终端画布绘制过程：尺寸同步、损伤跟踪、预计算与 GPUI 提交。

use super::{
    LayoutMetrics, PreparedTerminal, SEARCH_TICK_PIXELS, TerminalRenderState, TerminalView,
};
use crate::engine::TerminalEngine;
use crate::renderer::{DamageInfo, PaintRequest, TerminalRenderer};
use gpui_kit::*;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use transom_core::{
    BlockViewSettings, BlockViewStyle, TerminalControlHandle, TerminalProfile, TerminalSize,
};

/// 预绘制阶段的只读输入：外观重建、尺寸同步与检索快照都在这里完成。
struct CanvasPrepaintInputs {
    state: Arc<TerminalRenderState>,
    profile: TerminalProfile,
    control: TerminalControlHandle,
    search_open: bool,
    block_view: Option<BlockViewSettings>,
}

/// 绘制阶段的只读输入：只做输入挂接与已准备数据的提交。
struct CanvasPaintInputs {
    state: Arc<TerminalRenderState>,
    focus_handle: FocusHandle,
    entity: Entity<TerminalView>,
    ime_text: String,
    ime_marked_range: Option<Range<usize>>,
    cursor_blink_on: bool,
    block_style: BlockViewStyle,
}

impl TerminalView {
    /// 构造终端画布元素（包含 prepaint 计算网格与 paint 实际提交）。
    pub(super) fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let prepaint_inputs = CanvasPrepaintInputs {
            state: self.state.clone(),
            profile: self.profile.clone(),
            control: self.control.clone(),
            search_open: self.search_open,
            block_view: self.block_view,
        };
        let paint_inputs = CanvasPaintInputs {
            state: self.state.clone(),
            focus_handle: self.focus_handle.clone(),
            entity: cx.entity(),
            ime_text: self.ime_text.clone(),
            ime_marked_range: self.ime_marked_range.clone(),
            cursor_blink_on: self.cursor_blink_on,
            block_style: self
                .block_view
                .map(|settings| settings.style)
                .unwrap_or_default(),
        };
        canvas(
            move |bounds, window, cx| prepare_canvas(&prepaint_inputs, bounds, window, cx),
            move |bounds, prepared, window, cx| {
                submit_canvas_paint(&paint_inputs, bounds, prepared, window, cx);
            },
        )
        .size_full()
    }
}

/// 预绘制：重建外观、同步网格尺寸、取损伤集合并生成块布局与图像快照。
fn prepare_canvas(
    inputs: &CanvasPrepaintInputs,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> PreparedTerminal {
    let state = &inputs.state;
    // 外观变更（脏标记）在此重建：只有 prepaint 阶段持有窗口文本系统，
    // 度量（"M"/"中" 整形探测）必须在这里做。
    let mut renderer_rebuilt = false;
    if state.renderer_dirty.swap(false, Ordering::AcqRel) {
        *state.renderer.write() = TerminalRenderer::new(&inputs.profile, window, cx);
        renderer_rebuilt = true;
    }
    let renderer = state.renderer.read();
    let mut engine = state.engine.lock();
    let (size, resized) = sync_grid(&mut engine, &renderer, &inputs.control, bounds);
    let cols = size.cols;
    let rows = size.rows;
    // 刻度带行桶按像素高度折算，两种标记共用同一套分桶口径。
    let tick_rows = (f32::from(bounds.size.height) / SEARCH_TICK_PIXELS).max(1.0) as usize;
    // 检索重扫放在尺寸变更之后：缩放会重排折行、让快照作废，
    // 放在数据流路径里又会把刷屏变成卡顿，所以只在渲染帧里按冷却触发。
    let search = if inputs.search_open {
        engine.refresh_search(std::time::Instant::now());
        engine.search_paint(tick_rows)
    } else {
        None
    };
    let search_ticks = search.as_ref().is_some_and(|paint| !paint.ticks.is_empty());
    // 失败命令标记常驻：报错分布是用户随时想看的信息，不该因为搜索栏开着就消失。
    // 检索命中画在它之上，点击归属由"搜索栏是否开着"决定，见 `search_strip_click`。
    let failure_ticks = engine.failure_ticks(tick_rows);
    // 损伤集合必须在持锁的准备阶段取走并重置：它是"上次取走之后全部变化"
    // 的完整窗口。折行重排与外观重建让行号、字体度量整体失效，无论内核报
    // 了什么都提升为全量。
    let mut damage = engine.take_damage();
    if renderer_rebuilt || resized {
        damage = DamageInfo::Full;
    }
    // 刻度带只有在真的画了刻度时才登记靶区，否则会白白吃掉一列点击。
    let strip = (search_ticks || !failure_ticks.is_empty())
        .then(|| TerminalRenderer::search_strip_bounds(bounds));
    let metrics = build_metrics(&renderer, bounds, strip, cols as usize, rows as usize);
    *state.metrics.lock() = metrics;
    let layout = engine
        .build_block_layout(inputs.block_view.as_ref())
        .map(Arc::new);
    *state.block_layout.lock() = layout.clone();
    let graphics = engine.graphics_placements();
    PreparedTerminal {
        search,
        failure_ticks,
        damage,
        block_layout: layout,
        graphics,
    }
}

/// 绘制：挂接输入处理器，并把预绘制结果提交给渲染器。
fn submit_canvas_paint(
    inputs: &CanvasPaintInputs,
    bounds: Bounds<Pixels>,
    prepared: PreparedTerminal,
    window: &mut Window,
    cx: &mut App,
) {
    window.handle_input(
        &inputs.focus_handle,
        ElementInputHandler::new(bounds, inputs.entity.clone()),
        cx,
    );
    let engine = inputs.state.engine.lock();
    let renderer = inputs.state.renderer.read();
    let mut row_cache = inputs.state.row_cache.lock();
    let ime = if inputs.ime_text.is_empty() {
        None
    } else {
        Some((inputs.ime_text.as_str(), inputs.ime_marked_range.as_ref()))
    };
    renderer.paint(
        PaintRequest {
            bounds,
            term: engine.term(),
            ime,
            blink_visible: inputs.cursor_blink_on,
            search: prepared.search.as_ref(),
            failures: &prepared.failure_ticks,
            block_layout: prepared.block_layout.as_deref(),
            block_style: inputs.block_style,
            graphics: &prepared.graphics,
        },
        &mut row_cache,
        &prepared.damage,
        window,
        cx,
    );
}

/// 同步渲染器网格与 PTY 尺寸；返回尺寸以及是否发生了缩放。
fn sync_grid(
    engine: &mut TerminalEngine,
    renderer: &TerminalRenderer,
    control: &TerminalControlHandle,
    bounds: Bounds<Pixels>,
) -> (TerminalSize, bool) {
    let (cols, rows) = renderer.grid_size(bounds);
    let size = TerminalSize::new(cols, rows);
    if engine.size() == size {
        return (size, false);
    }
    engine.resize(size);
    // 尺寸同步失败会直接导致网格与 PTY 失配（提示符错位），
    // 因此不再静默忽略，按警告记录并给出排查方向。
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
    (size, true)
}

/// 由绘制边界与字形度量生成布局快照，供输入命中与绘制共用。
fn build_metrics(
    renderer: &TerminalRenderer,
    bounds: Bounds<Pixels>,
    strip: Option<Bounds<Pixels>>,
    cols: usize,
    rows: usize,
) -> LayoutMetrics {
    LayoutMetrics {
        origin: Point {
            x: bounds.origin.x + TerminalRenderer::TERMINAL_PADDING_X,
            y: bounds.origin.y + TerminalRenderer::TERMINAL_PADDING_Y,
        },
        cell_width: renderer.cell_width,
        cell_height: renderer.cell_height,
        cols,
        rows,
        search_strip: strip,
        bounds: Some(bounds),
    }
}
