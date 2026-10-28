//! 迷你历史时序走势图组件（Sparkline）。

use gpui_kit::base::h_flex;
use gpui_kit::*;

/// 历史采样走势的总槽位数（约 2 分钟时序窗口）。
pub(crate) const SPARKLINE_SLOTS: usize = 40;

/// 绘制百分比走势波形（数值范围 0.0 ~ 100.0），支持固定 40 时间槽位右对齐流动。
pub(crate) fn render_percent_sparkline(
    history: &[f32],
    height_px: f32,
    color: Hsla,
    muted_color: Hsla,
) -> AnyElement {
    let padding_count = SPARKLINE_SLOTS.saturating_sub(history.len());
    let mut slots = Vec::with_capacity(SPARKLINE_SLOTS);

    // 左侧未到达的时间槽位：显示极淡的底座参考标尺
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

    // 右侧真实时序数据：柱体圆角向上起伏
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

/// 绘制网络双向（下行与上行）速率对比波形，支持固定 40 槽位与成对胶囊双柱。
pub(crate) fn render_network_dual_sparkline(
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

    // 未采样槽位占位
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
