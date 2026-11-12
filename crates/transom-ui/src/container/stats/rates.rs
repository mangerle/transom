//! 速率换算、趋势窗口与通用卡片辅助。

use super::*;
use crate::theme::tokens::typography;
use gpui_kit::prelude::FluentBuilder;
use transom_ui_icons::AppIconElement;

pub(crate) fn stat_card(
    id: &'static str,
    icon: AppIcon,
    title: String,
    summary: AnyElement,
    content: AnyElement,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .id(id)
        .flex_1()
        .min_w_0()
        .min_h(px(160.0))
        .rounded(px(10.0))
        .bg(p.surface.ghost)
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_between()
                .px_3()
                .pt_3()
                .pb_1p5()
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .min_w_0()
                        .child(
                            AppIconElement::new(icon)
                                .size(px(14.0))
                                .text_color(p.muted_foreground),
                        )
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.muted_foreground)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(summary),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_2p5()
                .px_3()
                .pb_3()
                .child(content),
        )
        .into_any_element()
}

/// 核心指标大数字：26px tabular 等宽、字重 600；单位 12px 弱色贴右侧 baseline 对齐。
pub(crate) fn hero_stat(value: &str, unit: &str, color: Hsla, p: &Palette) -> AnyElement {
    h_flex()
        .items_baseline()
        .gap_1()
        .child(
            div()
                .font_family(p.mono_font_family.clone())
                .text_size(rems(1.625))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(28.0))
                .text_color(color)
                .child(value.to_string()),
        )
        .when(!unit.is_empty(), |this| {
            this.child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .font_weight(FontWeight::NORMAL)
                    .line_height(px(14.0))
                    .text_color(p.muted_foreground)
                    .child(unit.to_string()),
            )
        })
        .into_any_element()
}

/// 4px 细进度条：8% 弱色底轨道，填充采用语义主色。
pub(crate) fn usage_bar_slim(fraction: f32, color: Hsla, p: &Palette) -> AnyElement {
    let fraction = fraction.clamp(0.0, 1.0);
    div()
        .h(px(4.0))
        .w_full()
        .rounded_full()
        .bg(p.surface.faint)
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(color),
        )
        .into_any_element()
}

/// 将字节/秒速率拆分为数值和单位，供大字号指标独立排版。
pub(crate) fn split_rate(bytes_per_sec: u64) -> (String, String) {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes_per_sec as f64;
    if value < KB {
        (bytes_per_sec.to_string(), "B/s".to_string())
    } else if value < MB {
        (format!("{:.1}", value / KB), "KB/s".to_string())
    } else if value < GB {
        (format!("{:.1}", value / MB), "MB/s".to_string())
    } else {
        (format!("{:.1}", value / GB), "GB/s".to_string())
    }
}

pub(crate) fn summary_text(value: String, color: Hsla) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(value)
        .into_any_element()
}

/// 相邻采样的增量按秒折算：主机监控的速率口径是 `/s`，容器这里保持一致。
pub(crate) fn per_second(
    history: &[ContainerStats],
    pick: impl Fn(&ContainerStats) -> u64,
    seconds: f32,
) -> Vec<u64> {
    history
        .windows(2)
        .map(|pair| {
            let delta = pick(&pair[1]).saturating_sub(pick(&pair[0])) as f64;
            (delta / seconds as f64) as u64
        })
        .collect()
}

pub(crate) fn latest_interface_rates(
    history: &[ContainerStats],
    seconds: f32,
) -> Vec<(String, u64, u64)> {
    let Some(last) = history.last() else {
        return Vec::new();
    };
    let previous = history.len().checked_sub(2).map(|index| &history[index]);
    last.networks
        .iter()
        .map(|interface| {
            let (rx, tx) = previous
                .and_then(|sample| {
                    sample
                        .networks
                        .iter()
                        .find(|item| item.name == interface.name)
                })
                .map(|item| {
                    (
                        interface.rx_bytes.saturating_sub(item.rx_bytes),
                        interface.tx_bytes.saturating_sub(item.tx_bytes),
                    )
                })
                .unwrap_or((0, 0));
            (
                interface.name.clone(),
                (rx as f64 / seconds as f64) as u64,
                (tx as f64 / seconds as f64) as u64,
            )
        })
        .collect()
}

/// 卡片共用外观：走势图形态、窗口文案与调色板。
pub(crate) struct StatsChrome<'a> {
    pub(crate) chart: SparklineChartType,
    pub(crate) trend: &'a str,
    pub(crate) p: &'a Palette,
    /// 还没有第一帧数据：数值显示「—」，避免把占位的 0 当成真实值。
    pub(crate) waiting: bool,
}

/// 历史窗口文案：图表固定保留 40 个采样点，不把点数伪装成固定时长。
pub(crate) fn trend_window(_interval: SampleIntervalMode) -> String {
    rust_i18n::t!(
        "container.stats.window_samples",
        count = crate::container::types::STATS_HISTORY_LIMIT
    )
    .to_string()
}

pub(crate) fn trend_label(chrome: &StatsChrome) -> AnyElement {
    trend_label_with_peak(chrome, None)
}

pub(crate) fn trend_label_with_peak(chrome: &StatsChrome, peak: Option<f32>) -> AnyElement {
    let trend_text = rust_i18n::t!("container.stats.trend", window = chrome.trend).to_string();
    if let Some(peak_val) = peak {
        let peak_text =
            rust_i18n::t!("container.stats.peak", value = format!("{:.1}%", peak_val)).to_string();
        h_flex()
            .gap_2()
            .items_center()
            .text_xs()
            .text_color(chrome.p.muted_foreground)
            .child(trend_text)
            .child(div().child("·"))
            .child(peak_text)
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(chrome.p.muted_foreground)
            .child(trend_text)
            .into_any_element()
    }
}

/// 磁盘卡片数据：字段多且都是借用，收敛为结构体避免长参数列表。
pub(crate) struct DiskCardData<'a> {
    pub(crate) details: Option<&'a ContainerDetails>,
    pub(crate) read_rate: u64,
    pub(crate) write_rate: u64,
    pub(crate) read_rates: &'a [u64],
    pub(crate) write_rates: &'a [u64],
}
