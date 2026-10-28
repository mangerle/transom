//! 网络流量与上下行波形监控板块。

use crate::app::AppShell;
use crate::monitor::components::{CollapsibleCard, render_network_dual_sparkline};
use crate::monitor::types::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;

impl AppShell {
    /// 渲染网络流量监控卡片（包含双向波形与网卡明细）。
    pub(crate) fn render_network_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.network;

        // 计算当前总上行与总下行
        let (total_rx, total_tx) = stats
            .interfaces
            .iter()
            .fold((0u64, 0u64), |(rx, tx), iface| {
                (
                    rx.saturating_add(iface.rx_bytes_per_sec),
                    tx.saturating_add(iface.tx_bytes_per_sec),
                )
            });

        // 折叠态紧凑摘要
        let summary = h_flex()
            .gap_2()
            .text_xs()
            .child(
                div()
                    .text_color(p.primary)
                    .child(format!("↓ {}/s", format_size(total_rx))),
            )
            .child(
                div()
                    .text_color(p.sidebar_accent_foreground)
                    .child(format!("↑ {}/s", format_size(total_tx))),
            );

        // 展开态详细内容
        let mut content = v_flex().gap_2p5();

        // 双向流量波形走势（最近 40 个采样点，双线对比）
        let rx_hist: Vec<u64> = self.monitor.rx_history.iter().copied().collect();
        let tx_hist: Vec<u64> = self.monitor.tx_history.iter().copied().collect();
        if !rx_hist.is_empty() || !tx_hist.is_empty() {
            let rx_peak = rx_hist.iter().copied().max().unwrap_or(0);
            let tx_peak = tx_hist.iter().copied().max().unwrap_or(0);
            content = content.child(
                v_flex()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_color(p.muted_foreground)
                                            .child("总流量走势 (2分钟)"),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(
                                                div()
                                                    .w(px(6.0))
                                                    .h(px(6.0))
                                                    .rounded_full()
                                                    .bg(p.primary),
                                            )
                                            .child(
                                                div().text_xs().text_color(p.primary).child("下行"),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(
                                                div()
                                                    .w(px(6.0))
                                                    .h(px(6.0))
                                                    .rounded_full()
                                                    .bg(p.sidebar_accent_foreground),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(p.sidebar_accent_foreground)
                                                    .child("上行"),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_color(p.primary).child(format!(
                                        "↓ {}/s (峰值 {})",
                                        format_size(total_rx),
                                        format_size(rx_peak)
                                    )))
                                    .child(div().text_color(p.sidebar_accent_foreground).child(
                                        format!(
                                            "↑ {}/s (峰值 {})",
                                            format_size(total_tx),
                                            format_size(tx_peak)
                                        ),
                                    )),
                            ),
                    )
                    .child(render_network_dual_sparkline(
                        &rx_hist,
                        &tx_hist,
                        48.0,
                        p.primary,
                        p.sidebar_accent_foreground,
                        p.muted,
                    )),
            );
        }

        // 各网卡列表明细
        if stats.interfaces.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.no_data").to_string()),
            );
        }

        for interface in &stats.interfaces {
            content =
                content.child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .px_1()
                        .py_0p5()
                        .rounded_sm()
                        .hover(|s| s.bg(p.muted.opacity(0.3)))
                        .text_xs()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .child(interface.name.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_color(p.muted_foreground).child(format!(
                                    "↓ {}/s",
                                    format_size(interface.rx_bytes_per_sec)
                                )))
                                .child(div().text_color(p.muted_foreground).child(format!(
                                    "↑ {}/s",
                                    format_size(interface.tx_bytes_per_sec)
                                ))),
                        ),
                );
        }

        CollapsibleCard::new(
            "card-network",
            IconName::Network,
            rust_i18n::t!("monitor.network").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("network", cx);
            }),
        )
    }
}
