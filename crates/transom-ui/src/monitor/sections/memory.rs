//! 内存与交换分区监控板块。

use crate::app::AppShell;
use crate::monitor::components::{CollapsibleCard, render_percent_sparkline};
use crate::monitor::types::{format_size, usage_bar, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;

impl AppShell {
    /// 渲染内存与 Swap 监控卡片。
    pub(crate) fn render_memory_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.memory;

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
                                .child("交换分区 (Swap)"),
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

        // 内存历史走势波形（最近 40 个采样点）
        let history: Vec<f32> = self.monitor.mem_history.iter().copied().collect();
        if !history.is_empty() {
            let min_val = history.iter().copied().fold(f32::INFINITY, f32::min);
            let max_val = history.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let avg_val = history.iter().sum::<f32>() / history.len() as f32;
            let diff = max_val - min_val;
            let trend_badge = if diff < 1.0 {
                div()
                    .text_xs()
                    .text_color(p.sidebar_accent_foreground)
                    .child("[平稳]")
            } else if history.last().copied().unwrap_or(0.0)
                >= history.first().copied().unwrap_or(0.0)
            {
                div().text_xs().text_color(p.warning).child("[上升中]")
            } else {
                div().text_xs().text_color(p.primary).child("[下降中]")
            };

            content = content.child(
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
                                    .child(
                                        div().text_color(p.muted_foreground).child("走势 (2分钟)"),
                                    )
                                    .child(trend_badge),
                            )
                            .child(div().text_color(p.muted_foreground).child(format!(
                                "{:.1}% ~ {:.1}% (均 {:.1}%)",
                                min_val, max_val, avg_val
                            ))),
                    )
                    .child(render_percent_sparkline(
                        &history,
                        44.0,
                        mem_color,
                        p.muted,
                        self.monitor.chart_type,
                    )),
            );
        }

        CollapsibleCard::new(
            "card-mem",
            IconName::Server,
            rust_i18n::t!("monitor.mem").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("memory", cx);
            }),
        )
    }
}
