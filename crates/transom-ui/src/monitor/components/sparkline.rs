//! 迷你历史时序走势图组件（Sparkline）。

use crate::monitor::types::SparklineChartType;
use gpui_kit::base::h_flex;
use gpui_kit::gpui::{PathBuilder, fill};
use gpui_kit::*;

/// 历史采样走势的总槽位数（约 2 分钟时序窗口）。
pub(crate) const SPARKLINE_SLOTS: usize = 40;

/// 绘制百分比走势波形（数值范围 0.0 ~ 100.0），支持折线面积图与柱状图。
pub(crate) fn render_percent_sparkline(
    history: &[f32],
    height_px: f32,
    color: Hsla,
    muted_color: Hsla,
    chart_type: SparklineChartType,
) -> AnyElement {
    match chart_type {
        SparklineChartType::Bar => render_percent_bars(history, height_px, color, muted_color),
        SparklineChartType::Line => render_percent_line(history, height_px, color, muted_color),
    }
}

/// 柱状形态绘制百分比走势。
fn render_percent_bars(
    history: &[f32],
    height_px: f32,
    color: Hsla,
    muted_color: Hsla,
) -> AnyElement {
    let padding_count = SPARKLINE_SLOTS.saturating_sub(history.len());
    let mut slots = Vec::with_capacity(SPARKLINE_SLOTS);

    for _ in 0..padding_count {
        slots.push(
            div()
                .flex_1()
                .min_w_0()
                .h(px(1.5))
                .rounded_full()
                .bg(muted_color.opacity(0.2)),
        );
    }

    let start_idx = history.len().saturating_sub(SPARKLINE_SLOTS);
    for &val in &history[start_idx..] {
        let fraction = (val / 100.0).clamp(0.0, 1.0);
        let bar_h = (height_px * fraction).max(2.0);
        slots.push(
            div()
                .flex_1()
                .min_w_0()
                .h(px(bar_h))
                .rounded_t_xs()
                .bg(color),
        );
    }

    h_flex()
        .h(px(height_px))
        .w_full()
        .overflow_hidden()
        .items_end()
        .gap(px(1.5))
        .p_1p5()
        .rounded_sm()
        .bg(muted_color.opacity(0.18))
        .border_1()
        .border_color(muted_color.opacity(0.3))
        .children(slots)
        .into_any_element()
}

/// 折线面积图形态绘制百分比走势。
fn render_percent_line(
    history: &[f32],
    height_px: f32,
    color: Hsla,
    muted_color: Hsla,
) -> AnyElement {
    let history = history.to_vec();
    div()
        .h(px(height_px))
        .w_full()
        .overflow_hidden()
        .p_1p5()
        .rounded_sm()
        .bg(muted_color.opacity(0.18))
        .border_1()
        .border_color(muted_color.opacity(0.3))
        .child(
            canvas(
                move |_bounds, _window, _cx| {},
                move |bounds, _, window, _cx| {
                    paint_percent_line(bounds, &history, color, muted_color, window);
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// 实际绘制百分比平滑折线与渐变面积。
fn paint_percent_line(
    bounds: Bounds<Pixels>,
    history: &[f32],
    color: Hsla,
    muted_color: Hsla,
    window: &mut Window,
) {
    let w = bounds.size.width;
    let h = bounds.size.height;
    if w <= px(0.0) || h <= px(0.0) {
        return;
    }

    let pad_y = px(2.5);
    let baseline_y = bounds.origin.y + h - pad_y;
    let effective_h = (h - pad_y * 2.0).max(px(1.0));

    if history.is_empty() {
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(w, px(1.0)));
        window.paint_quad(fill(line_rect, muted_color.opacity(0.2)));
        return;
    }

    let data_len = history.len().min(SPARKLINE_SLOTS);
    let padding_slots = SPARKLINE_SLOTS.saturating_sub(data_len);
    let step_x = w / ((SPARKLINE_SLOTS.max(1) - 1) as f32);

    // 未采样时段绘制基线底座
    if padding_slots > 0 {
        let pad_w = step_x * (padding_slots as f32);
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(pad_w, px(1.0)));
        window.paint_quad(fill(line_rect, muted_color.opacity(0.2)));
    }

    let start_idx = history.len().saturating_sub(SPARKLINE_SLOTS);
    let mut points = Vec::with_capacity(data_len);
    for (i, &val) in history[start_idx..].iter().enumerate() {
        let slot_idx = padding_slots + i;
        let x = bounds.origin.x + step_x * (slot_idx as f32);
        let fraction = (val / 100.0).clamp(0.0, 1.0);
        let y = baseline_y - effective_h * fraction;
        points.push(point(x, y));
    }

    paint_area_and_stroke(&points, baseline_y, color, window);
}

/// 绘制单个序列的面积填充与平滑描边。
fn paint_area_and_stroke(
    points: &[Point<Pixels>],
    baseline_y: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    if points.is_empty() {
        return;
    }

    // 1. 面积半透明填充
    let mut fill_builder = PathBuilder::fill();
    fill_builder.move_to(point(points[0].x, baseline_y));
    for pt in points {
        fill_builder.line_to(*pt);
    }
    fill_builder.line_to(point(points[points.len() - 1].x, baseline_y));
    if let Ok(fill_path) = fill_builder.build() {
        window.paint_path(fill_path, color.opacity(0.18));
    }

    // 2. 1.5px 折线描边
    let mut stroke_builder = PathBuilder::stroke(px(1.5));
    stroke_builder.move_to(points[0]);
    for pt in &points[1..] {
        stroke_builder.line_to(*pt);
    }
    if let Ok(stroke_path) = stroke_builder.build() {
        window.paint_path(stroke_path, color);
    }

    // 3. 最新采样点高亮端点（2px 微型强调点）
    let last = points[points.len() - 1];
    let dot_rect = Bounds::new(
        point(last.x - px(2.0), last.y - px(2.0)),
        gpui_kit::gpui::size(px(4.0), px(4.0)),
    );
    window.paint_quad(fill(dot_rect, color));
}

/// 绘制网络双向（下行与上行）速率对比波形，支持折线面积图与柱状图。
pub(crate) fn render_network_dual_sparkline(
    rx_history: &[u64],
    tx_history: &[u64],
    height_px: f32,
    rx_color: Hsla,
    tx_color: Hsla,
    muted_color: Hsla,
    chart_type: SparklineChartType,
) -> AnyElement {
    match chart_type {
        SparklineChartType::Bar => render_network_bars(
            rx_history,
            tx_history,
            height_px,
            rx_color,
            tx_color,
            muted_color,
        ),
        SparklineChartType::Line => render_network_line(
            rx_history,
            tx_history,
            height_px,
            rx_color,
            tx_color,
            muted_color,
        ),
    }
}

/// 柱状形态绘制网络双向对比波形。
fn render_network_bars(
    rx_history: &[u64],
    tx_history: &[u64],
    height_px: f32,
    rx_color: Hsla,
    tx_color: Hsla,
    muted_color: Hsla,
) -> AnyElement {
    let rx_peak = rx_history.iter().copied().max().unwrap_or(1).max(1);
    let tx_peak = tx_history.iter().copied().max().unwrap_or(1).max(1);
    let peak = rx_peak.max(tx_peak);

    let len = rx_history.len().max(tx_history.len());
    let padding_count = SPARKLINE_SLOTS.saturating_sub(len);
    let mut slots = Vec::with_capacity(SPARKLINE_SLOTS);

    for _ in 0..padding_count {
        slots.push(
            div()
                .flex_1()
                .min_w_0()
                .h(px(1.5))
                .rounded_full()
                .bg(muted_color.opacity(0.2)),
        );
    }

    let start_idx = len.saturating_sub(SPARKLINE_SLOTS);
    for i in start_idx..len {
        let rx = rx_history.get(i).copied().unwrap_or(0);
        let tx = tx_history.get(i).copied().unwrap_or(0);
        let rx_frac = (rx as f32 / peak as f32).clamp(0.0, 1.0);
        let tx_frac = (tx as f32 / peak as f32).clamp(0.0, 1.0);

        let rx_h = (height_px * rx_frac).max(2.0);
        let tx_h = (height_px * tx_frac).max(2.0);

        slots.push(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_end()
                .gap(px(1.0))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .h(px(rx_h))
                        .rounded_t_xs()
                        .bg(rx_color),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .h(px(tx_h))
                        .rounded_t_xs()
                        .bg(tx_color),
                ),
        );
    }

    h_flex()
        .h(px(height_px))
        .w_full()
        .overflow_hidden()
        .items_end()
        .gap(px(1.5))
        .p_1p5()
        .rounded_sm()
        .bg(muted_color.opacity(0.18))
        .border_1()
        .border_color(muted_color.opacity(0.3))
        .children(slots)
        .into_any_element()
}

/// 折线面积图形态绘制网络双向对比波形。
fn render_network_line(
    rx_history: &[u64],
    tx_history: &[u64],
    height_px: f32,
    rx_color: Hsla,
    tx_color: Hsla,
    muted_color: Hsla,
) -> AnyElement {
    let rx_history = rx_history.to_vec();
    let tx_history = tx_history.to_vec();
    div()
        .h(px(height_px))
        .w_full()
        .overflow_hidden()
        .p_1p5()
        .rounded_sm()
        .bg(muted_color.opacity(0.18))
        .border_1()
        .border_color(muted_color.opacity(0.3))
        .child(
            canvas(
                move |_bounds, _window, _cx| {},
                move |bounds, _, window, _cx| {
                    paint_network_lines(
                        bounds,
                        &rx_history,
                        &tx_history,
                        rx_color,
                        tx_color,
                        muted_color,
                        window,
                    );
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// 实际绘制网络双向平滑折线与重叠面积。
fn paint_network_lines(
    bounds: Bounds<Pixels>,
    rx_history: &[u64],
    tx_history: &[u64],
    rx_color: Hsla,
    tx_color: Hsla,
    muted_color: Hsla,
    window: &mut Window,
) {
    let w = bounds.size.width;
    let h = bounds.size.height;
    if w <= px(0.0) || h <= px(0.0) {
        return;
    }

    let pad_y = px(2.5);
    let baseline_y = bounds.origin.y + h - pad_y;
    let effective_h = (h - pad_y * 2.0).max(px(1.0));

    let rx_peak = rx_history.iter().copied().max().unwrap_or(1).max(1);
    let tx_peak = tx_history.iter().copied().max().unwrap_or(1).max(1);
    let peak = rx_peak.max(tx_peak);

    let len = rx_history.len().max(tx_history.len());
    if len == 0 {
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(w, px(1.0)));
        window.paint_quad(fill(line_rect, muted_color.opacity(0.2)));
        return;
    }

    let data_len = len.min(SPARKLINE_SLOTS);
    let padding_slots = SPARKLINE_SLOTS.saturating_sub(data_len);
    let step_x = w / ((SPARKLINE_SLOTS.max(1) - 1) as f32);

    if padding_slots > 0 {
        let pad_w = step_x * (padding_slots as f32);
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(pad_w, px(1.0)));
        window.paint_quad(fill(line_rect, muted_color.opacity(0.2)));
    }

    let start_idx = len.saturating_sub(SPARKLINE_SLOTS);
    let mut rx_points = Vec::with_capacity(data_len);
    let mut tx_points = Vec::with_capacity(data_len);

    for (offset, i) in (start_idx..len).enumerate() {
        let slot_idx = padding_slots + offset;
        let x = bounds.origin.x + step_x * (slot_idx as f32);

        let rx = rx_history.get(i).copied().unwrap_or(0);
        let tx = tx_history.get(i).copied().unwrap_or(0);
        let rx_frac = (rx as f32 / peak as f32).clamp(0.0, 1.0);
        let tx_frac = (tx as f32 / peak as f32).clamp(0.0, 1.0);

        rx_points.push(point(x, baseline_y - effective_h * rx_frac));
        tx_points.push(point(x, baseline_y - effective_h * tx_frac));
    }

    // 先画下行面积与折线，再画上行面积与折线，半透明自然交叠
    paint_area_and_stroke(&rx_points, baseline_y, rx_color, window);
    paint_area_and_stroke(&tx_points, baseline_y, tx_color, window);
}
