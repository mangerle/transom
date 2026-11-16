//! 迷你历史时序走势图通用组件（Sparkline）。
//!
//! 设计原理（Why）：
//! - 主机监控与容器统计原先各自维护一份 400 余行的走势图实现，绘图几何与
//!   Canvas 渲染逻辑几乎完全一致，仅槽位上限（40 vs 120）与高度/填充策略不同；
//! - 统一收敛到 `widgets` 后，以 [`SparklineConfig`] 与 [`SparklineLayout`] 参数化
//!   差异，消除跨模块重复绘制代码并收敛多参数签名。

// 设计令牌例外：走势图 canvas 的像素几何属于数据可视化绘制，保持物理像素。
use gpui_kit::base::h_flex;
use gpui_kit::gpui::{PathBuilder, fill};
use gpui_kit::*;

/// 监控走势图渲染形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SparklineChartType {
    /// 平滑折线面积图（默认）。
    #[default]
    Line,
    /// 紧凑柱状图。
    Bar,
}

/// 走势图容器尺寸与样本填充策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SparklineLayout {
    /// 固定高度，未采样时段在左侧补齐占位槽位或基线（主机监控使用）。
    FixedWindow,
    /// 弹性高度，仅按已有样本铺满容器宽度（容器统计卡片使用）。
    FlexFill,
}

/// 走势图渲染配置。
#[derive(Debug, Clone, Copy)]
pub(crate) struct SparklineConfig {
    /// 最大展示槽位数。
    pub(crate) slots: usize,
    /// 基准高度（像素）。
    pub(crate) height_px: f32,
    /// 边框与基线使用的弱化颜色。
    pub(crate) muted_color: Hsla,
    /// 渲染形态（折线或柱状）。
    pub(crate) chart_type: SparklineChartType,
    /// 尺寸与样本填充策略。
    pub(crate) layout: SparklineLayout,
}

/// 绘制百分比走势波形（数值范围 0.0 ~ 100.0），支持折线面积图与柱状图。
pub(crate) fn render_percent_sparkline(
    history: &[f32],
    color: Hsla,
    config: SparklineConfig,
) -> AnyElement {
    match config.chart_type {
        SparklineChartType::Bar => render_percent_bars(history, color, config),
        SparklineChartType::Line => render_percent_line(history, color, config),
    }
}

/// 绘制网络双向（下行与上行）速率对比波形，支持折线面积图与柱状图。
pub(crate) fn render_network_dual_sparkline(
    rx_history: &[u64],
    tx_history: &[u64],
    rx_color: Hsla,
    tx_color: Hsla,
    config: SparklineConfig,
) -> AnyElement {
    match config.chart_type {
        SparklineChartType::Bar => {
            render_network_bars(rx_history, tx_history, rx_color, tx_color, config)
        }
        SparklineChartType::Line => {
            render_network_line(rx_history, tx_history, rx_color, tx_color, config)
        }
    }
}

/// 按布局策略为外层容器应用固定高度或弹性高度。
fn apply_container_box(el: Div, config: SparklineConfig) -> Div {
    let base = match config.layout {
        SparklineLayout::FixedWindow => el.h(px(config.height_px)),
        SparklineLayout::FlexFill => el.flex_1().min_h(px(config.height_px)),
    };
    base.w_full()
        .overflow_hidden()
        .p_1p5()
        .rounded_sm()
        .bg(config.muted_color.opacity(0.18))
        .border_1()
        .border_color(config.muted_color.opacity(0.3))
}

/// 无样本时的占位横条（仅 `FlexFill` 柱状形态使用）。
fn render_empty_flex_bar(config: SparklineConfig) -> AnyElement {
    apply_container_box(h_flex(), config)
        .items_end()
        .child(
            div()
                .w_full()
                .h(px(1.5))
                .rounded_full()
                .bg(config.muted_color.opacity(0.2)),
        )
        .into_any_element()
}

/// 构造单个占位的空槽位。
fn empty_slot_bar(muted_color: Hsla) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h(px(1.5))
        .rounded_full()
        .bg(muted_color.opacity(0.2))
}

/// 柱状形态绘制百分比走势。
fn render_percent_bars(history: &[f32], color: Hsla, config: SparklineConfig) -> AnyElement {
    let data_len = history.len().min(config.slots);
    if matches!(config.layout, SparklineLayout::FlexFill) && data_len == 0 {
        return render_empty_flex_bar(config);
    }

    let padding_count = match config.layout {
        SparklineLayout::FixedWindow => config.slots.saturating_sub(history.len()),
        SparklineLayout::FlexFill => 0,
    };
    let mut slots = Vec::with_capacity(data_len + padding_count);
    for _ in 0..padding_count {
        slots.push(empty_slot_bar(config.muted_color));
    }

    let start_idx = history.len().saturating_sub(config.slots);
    for &val in &history[start_idx..] {
        let slot = match config.layout {
            SparklineLayout::FixedWindow => {
                let fraction = (val / 100.0).clamp(0.0, 1.0);
                let bar_h = (config.height_px * fraction).max(2.0);
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(bar_h))
                    .rounded_t_xs()
                    .bg(color)
            }
            SparklineLayout::FlexFill => {
                let fraction = (val / 100.0).clamp(0.02, 1.0);
                div().flex_1().min_w_0().h_full().flex().items_end().child(
                    div()
                        .w_full()
                        .min_w_0()
                        .h(relative(fraction))
                        .rounded_t_xs()
                        .bg(color),
                )
            }
        };
        slots.push(slot);
    }

    apply_container_box(h_flex(), config)
        .items_end()
        .gap(px(1.5))
        .children(slots)
        .into_any_element()
}

/// 折线面积图形态绘制百分比走势。
fn render_percent_line(history: &[f32], color: Hsla, config: SparklineConfig) -> AnyElement {
    let history = history.to_vec();
    apply_container_box(div(), config)
        .child(
            canvas(
                move |_bounds, _window, _cx| {},
                move |bounds, _, window, _cx| {
                    paint_percent_line(bounds, &history, color, config, window);
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
    config: SparklineConfig,
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
    let data_len = history.len().min(config.slots);

    match config.layout {
        SparklineLayout::FixedWindow if history.is_empty() => {
            let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(w, px(1.0)));
            window.paint_quad(fill(line_rect, config.muted_color.opacity(0.2)));
            return;
        }
        SparklineLayout::FlexFill if data_len <= 1 => {
            let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(w, px(1.0)));
            window.paint_quad(fill(line_rect, config.muted_color.opacity(0.2)));
            if data_len == 1 {
                let fraction = (history[0] / 100.0).clamp(0.0, 1.0);
                let y = baseline_y - effective_h * fraction;
                let single_rect = Bounds::new(point(bounds.origin.x, y), size(w, px(1.5)));
                window.paint_quad(fill(single_rect, color));
            }
            return;
        }
        _ => {}
    }

    let (padding_slots, step_x) = compute_x_step(w, data_len, config);
    if padding_slots > 0 {
        let pad_w = step_x * (padding_slots as f32);
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(pad_w, px(1.0)));
        window.paint_quad(fill(line_rect, config.muted_color.opacity(0.2)));
    }

    let start_idx = history.len().saturating_sub(config.slots);
    let mut points = Vec::with_capacity(data_len);
    for (i, &val) in history[start_idx..].iter().enumerate() {
        let x = bounds.origin.x + step_x * ((padding_slots + i) as f32);
        let fraction = (val / 100.0).clamp(0.0, 1.0);
        let y = baseline_y - effective_h * fraction;
        points.push(point(x, y));
    }

    paint_area_and_stroke(&points, baseline_y, color, window);
}

/// 柱状形态绘制网络双向对比波形。
fn render_network_bars(
    rx_history: &[u64],
    tx_history: &[u64],
    rx_color: Hsla,
    tx_color: Hsla,
    config: SparklineConfig,
) -> AnyElement {
    let peak = compute_network_peak(rx_history, tx_history);
    let len = rx_history.len().max(tx_history.len());
    let data_len = len.min(config.slots);
    if matches!(config.layout, SparklineLayout::FlexFill) && data_len == 0 {
        return render_empty_flex_bar(config);
    }

    let padding_count = match config.layout {
        SparklineLayout::FixedWindow => config.slots.saturating_sub(len),
        SparklineLayout::FlexFill => 0,
    };
    let mut slots = Vec::with_capacity(data_len + padding_count);
    for _ in 0..padding_count {
        slots.push(empty_slot_bar(config.muted_color));
    }

    let start_idx = len.saturating_sub(config.slots);
    for i in start_idx..len {
        let rx = rx_history.get(i).copied().unwrap_or(0);
        let tx = tx_history.get(i).copied().unwrap_or(0);
        let (rx_bar, tx_bar) = match config.layout {
            SparklineLayout::FixedWindow => {
                let rx_h = (config.height_px * (rx as f32 / peak as f32).clamp(0.0, 1.0)).max(2.0);
                let tx_h = (config.height_px * (tx as f32 / peak as f32).clamp(0.0, 1.0)).max(2.0);
                (
                    div()
                        .w_full()
                        .min_w_0()
                        .h(px(rx_h))
                        .rounded_t_xs()
                        .bg(rx_color),
                    div()
                        .w_full()
                        .min_w_0()
                        .h(px(tx_h))
                        .rounded_t_xs()
                        .bg(tx_color),
                )
            }
            SparklineLayout::FlexFill => {
                let rx_frac = (rx as f32 / peak as f32).clamp(0.02, 1.0);
                let tx_frac = (tx as f32 / peak as f32).clamp(0.02, 1.0);
                (
                    div()
                        .w_full()
                        .min_w_0()
                        .h(relative(rx_frac))
                        .rounded_t_xs()
                        .bg(rx_color),
                    div()
                        .w_full()
                        .min_w_0()
                        .h(relative(tx_frac))
                        .rounded_t_xs()
                        .bg(tx_color),
                )
            }
        };

        slots.push(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_end()
                .gap(px(1.0))
                .child(rx_bar)
                .child(tx_bar),
        );
    }

    apply_container_box(h_flex(), config)
        .items_end()
        .gap(px(1.5))
        .children(slots)
        .into_any_element()
}

/// 折线面积图形态绘制网络双向对比波形。
fn render_network_line(
    rx_history: &[u64],
    tx_history: &[u64],
    rx_color: Hsla,
    tx_color: Hsla,
    config: SparklineConfig,
) -> AnyElement {
    let rx_history = rx_history.to_vec();
    let tx_history = tx_history.to_vec();
    apply_container_box(div(), config)
        .child(
            canvas(
                move |_bounds, _window, _cx| {},
                move |bounds, _, window, _cx| {
                    paint_network_lines(
                        bounds,
                        NetworkSeries {
                            rx_history: &rx_history,
                            tx_history: &tx_history,
                            rx_color,
                            tx_color,
                        },
                        config,
                        window,
                    );
                },
            )
            .size_full(),
        )
        .into_any_element()
}

struct NetworkSeries<'a> {
    rx_history: &'a [u64],
    tx_history: &'a [u64],
    rx_color: Hsla,
    tx_color: Hsla,
}

/// 实际绘制网络双向平滑折线与重叠面积。
fn paint_network_lines(
    bounds: Bounds<Pixels>,
    series: NetworkSeries<'_>,
    config: SparklineConfig,
    window: &mut Window,
) {
    let NetworkSeries {
        rx_history,
        tx_history,
        rx_color,
        tx_color,
    } = series;
    let w = bounds.size.width;
    let h = bounds.size.height;
    if w <= px(0.0) || h <= px(0.0) {
        return;
    }

    let pad_y = px(2.5);
    let baseline_y = bounds.origin.y + h - pad_y;
    let effective_h = (h - pad_y * 2.0).max(px(1.0));
    let peak = compute_network_peak(rx_history, tx_history);

    let len = rx_history.len().max(tx_history.len());
    let data_len = len.min(config.slots);
    let no_curve = match config.layout {
        SparklineLayout::FixedWindow => len == 0,
        SparklineLayout::FlexFill => data_len <= 1,
    };
    if no_curve {
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(w, px(1.0)));
        window.paint_quad(fill(line_rect, config.muted_color.opacity(0.2)));
        return;
    }

    let (padding_slots, step_x) = compute_x_step(w, data_len, config);
    if padding_slots > 0 {
        let pad_w = step_x * (padding_slots as f32);
        let line_rect = Bounds::new(point(bounds.origin.x, baseline_y), size(pad_w, px(1.0)));
        window.paint_quad(fill(line_rect, config.muted_color.opacity(0.2)));
    }

    let start_idx = len.saturating_sub(config.slots);
    let mut rx_points = Vec::with_capacity(data_len);
    let mut tx_points = Vec::with_capacity(data_len);
    for (offset, i) in (start_idx..len).enumerate() {
        let x = bounds.origin.x + step_x * ((padding_slots + offset) as f32);
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

/// 计算折线左侧占位槽位数与水平步长。
fn compute_x_step(w: Pixels, data_len: usize, config: SparklineConfig) -> (usize, Pixels) {
    match config.layout {
        SparklineLayout::FixedWindow => {
            let padding_slots = config.slots.saturating_sub(data_len);
            let step_x = w / ((config.slots.max(2) - 1) as f32);
            (padding_slots, step_x)
        }
        SparklineLayout::FlexFill => (0, w / ((data_len.max(2) - 1) as f32)),
    }
}

/// 取收发历史中的峰值作为归一化分母（至少为 1 避免除零）。
fn compute_network_peak(rx_history: &[u64], tx_history: &[u64]) -> u64 {
    let rx_peak = rx_history.iter().copied().max().unwrap_or(1).max(1);
    let tx_peak = tx_history.iter().copied().max().unwrap_or(1).max(1);
    rx_peak.max(tx_peak)
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

    let mut fill_builder = PathBuilder::fill();
    fill_builder.move_to(point(points[0].x, baseline_y));
    for pt in points {
        fill_builder.line_to(*pt);
    }
    fill_builder.line_to(point(points[points.len() - 1].x, baseline_y));
    if let Ok(fill_path) = fill_builder.build() {
        window.paint_path(fill_path, color.opacity(0.18));
    }

    let mut stroke_builder = PathBuilder::stroke(px(1.5));
    stroke_builder.move_to(points[0]);
    for pt in &points[1..] {
        stroke_builder.line_to(*pt);
    }
    if let Ok(stroke_path) = stroke_builder.build() {
        window.paint_path(stroke_path, color);
    }

    let last = points[points.len() - 1];
    let dot_rect = Bounds::new(
        point(last.x - px(2.0), last.y - px(2.0)),
        gpui_kit::gpui::size(px(4.0), px(4.0)),
    );
    window.paint_quad(fill(dot_rect, color));
}
