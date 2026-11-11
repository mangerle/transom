//! 内存与交换分区监控板块。

use crate::monitor::components::{CollapsibleCard, render_percent_sparkline};
use crate::monitor::types::{format_size, usage_bar, usage_color};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;

use crate::monitor::MonitorView;

impl MonitorView {
    /// 渲染内存与 Swap 监控卡片。
    pub(crate) fn render_memory_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.state.collapse.memory;

        let mem_fraction = if stats.mem_total_bytes == 0 {
            0.0
        } else {
            stats.mem_used_bytes as f32 / stats.mem_total_bytes as f32
        };
        let mem_color = usage_color(mem_fraction * 100.0, p);

        // 折叠态紧凑摘要
        let summary = h_flex()
            .gap_2()
            .text_xs()
            .child(div().text_color(p.muted_foreground).child(format!(
                "{}/{}",
                format_size(stats.mem_used_bytes),
                format_size(stats.mem_total_bytes)
            )))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(mem_color)
                    .child(format!("{:.0}%", mem_fraction * 100.0)),
            );

        // 展开态详细内容
        let mut content = v_flex().gap_2p5();

        content = content.child(render_memory_usage(stats, mem_fraction, mem_color, p));

        if let Some(trend) = self.render_memory_trend(mem_color, p) {
            content = content.child(trend);
        }

        CollapsibleCard::new(
            "card-mem",
            AppIcon::Custom(CustomIcon::MEMORY),
            rust_i18n::t!("monitor.mem").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .summary_always(true)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                let expanded = this.toggle_collapse("memory");
                let collapse = this.collapse_snapshot();
                cx.notify();
                cx.emit(crate::monitor::MonitorPanelEvent::CollapseChanged {
                    collapse,
                    expanded_processes: expanded,
                });
                cx.notify();
            }),
        )
    }

    /// 内存历史走势：区间统计、趋势徽标与波形；无采样返回 None。
    fn render_memory_trend(&self, mem_color: Hsla, p: &Palette) -> Option<AnyElement> {
        let history: Vec<f32> = self.state.mem_history.iter().copied().collect();
        if history.is_empty() {
            return None;
        }
        let min_val = history.iter().copied().fold(f32::INFINITY, f32::min);
        let max_val = history.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let avg_val = history.iter().sum::<f32>() / history.len() as f32;
        let diff = max_val - min_val;
        let trend_badge = if diff < 1.0 {
            div()
                .text_xs()
                .text_color(p.sidebar_accent_foreground)
                .child(rust_i18n::t!("monitor.memory_section.trend_flat").to_string())
        } else if history.last().copied().unwrap_or(0.0) >= history.first().copied().unwrap_or(0.0)
        {
            div()
                .text_xs()
                .text_color(p.warning)
                .child(rust_i18n::t!("monitor.memory_section.trend_rising").to_string())
        } else {
            div()
                .text_xs()
                .text_color(p.primary)
                .child(rust_i18n::t!("monitor.memory_section.trend_falling").to_string())
        };
        Some(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .text_xs()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(div().text_color(p.muted_foreground).child(
                                    rust_i18n::t!("monitor.memory_section.trend_title").to_string(),
                                ))
                                .child(trend_badge),
                        )
                        .child(
                            div().text_color(p.muted_foreground).child(
                                rust_i18n::t!(
                                    "monitor.memory_section.trend_range",
                                    min = format!("{min_val:.1}"),
                                    max = format!("{max_val:.1}"),
                                    avg = format!("{avg_val:.1}")
                                )
                                .to_string(),
                            ),
                        ),
                )
                .child(render_percent_sparkline(
                    &history,
                    44.0,
                    mem_color,
                    p.muted,
                    self.state.chart_type,
                ))
                .into_any_element(),
        )
    }
}

/// 物理内存与 Swap 进度条。
fn render_memory_usage(
    stats: &SystemStats,
    mem_fraction: f32,
    mem_color: Hsla,
    p: &Palette,
) -> AnyElement {
    let mut content = v_flex().gap_2p5();
    // 物理内存 RAM 进度条
    content = content
        .child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(
                    div()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("monitor.mem").to_string()),
                )
                .child(div().text_color(p.foreground).child(format!(
                    "{} / {} ({:.1}%)",
                    format_size(stats.mem_used_bytes),
                    format_size(stats.mem_total_bytes),
                    mem_fraction * 100.0
                ))),
        )
        .child(usage_bar(mem_fraction, mem_color, p));

    // Swap 交换分区（若配置了 Swap）
    if stats.swap_total_bytes > 0 {
        let swap_fraction = stats.swap_used_bytes as f32 / stats.swap_total_bytes as f32;
        let swap_color = usage_color(swap_fraction * 100.0, p);
        content = content
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("monitor.memory_section.swap").to_string()),
                    )
                    .child(div().text_color(p.foreground).child(format!(
                        "{} / {} ({:.1}%)",
                        format_size(stats.swap_used_bytes),
                        format_size(stats.swap_total_bytes),
                        swap_fraction * 100.0
                    ))),
            )
            .child(usage_bar(swap_fraction, swap_color, p));
    }

    content.into_any_element()
}
