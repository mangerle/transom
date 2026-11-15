//! 监控面板迷你走势图预设：委托给 [`crate::sparkline`] 通用组件。

use crate::sparkline::{self, SparklineChartType, SparklineConfig, SparklineLayout};
use gpui_kit::{AnyElement, Hsla};

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
    sparkline::render_percent_sparkline(
        history,
        color,
        SparklineConfig {
            slots: SPARKLINE_SLOTS,
            height_px,
            muted_color,
            chart_type,
            layout: SparklineLayout::FixedWindow,
        },
    )
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
    sparkline::render_network_dual_sparkline(
        rx_history,
        tx_history,
        rx_color,
        tx_color,
        SparklineConfig {
            slots: SPARKLINE_SLOTS,
            height_px,
            muted_color,
            chart_type,
            layout: SparklineLayout::FixedWindow,
        },
    )
}
