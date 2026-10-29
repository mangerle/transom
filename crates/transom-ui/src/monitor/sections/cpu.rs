//! CPU 监控板块（包含负载、细分状态、走势与核心图表）。

use crate::app::AppShell;
use crate::monitor::components::{CollapsibleCard, render_percent_sparkline};
use crate::monitor::types::{usage_bar, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;

impl AppShell {
    /// 渲染 CPU 处理器监控卡片。
    pub(crate) fn render_cpu_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.cpu;
        let color = usage_color(stats.cpu_percent, p);

        // 折叠态紧凑摘要
        let summary = h_flex()
            .gap_2()
            .text_xs()
            .child(
                div().text_color(p.muted_foreground).child(
                    rust_i18n::t!(
                        "monitor.cpu_section.load_summary",
                        load = format!("{:.2}", stats.load_avg.0)
                    )
                    .to_string(),
                ),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(format!("{:.0}%", stats.cpu_percent)),
            );

        // 展开态详细内容
        let mut content = v_flex().gap_2p5();

        // 顶层总进度条与即时百分比
        content = content
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("monitor.cpu").to_string()),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(format!("{:.1}%", stats.cpu_percent)),
                    ),
            )
            .child(usage_bar(stats.cpu_percent / 100.0, color, p));

        // 系统平均负载 (1m, 5m, 15m)
        content = content.child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(
                    div()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("monitor.load_avg").to_string()),
                )
                .child(div().text_color(p.foreground).child(format!(
                    "{:.2}, {:.2}, {:.2}",
                    stats.load_avg.0, stats.load_avg.1, stats.load_avg.2
                ))),
        );

        // 细分状态 (User / System / I/O Wait)
        content = content.child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("monitor.cpu_section.user").to_string()),
                        )
                        .child(
                            div()
                                .text_color(p.foreground)
                                .child(format!("{:.1}%", stats.cpu_user_percent)),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("monitor.cpu_section.system").to_string()),
                        )
                        .child(
                            div()
                                .text_color(p.foreground)
                                .child(format!("{:.1}%", stats.cpu_system_percent)),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("monitor.cpu_section.iowait").to_string()),
                        )
                        .child(
                            div()
                                .text_color(if stats.cpu_iowait_percent > 20.0 {
                                    p.danger
                                } else {
                                    p.foreground
                                })
                                .child(format!("{:.1}%", stats.cpu_iowait_percent)),
                        ),
                ),
        );

        // 历史走势波形（最近 40 个采样点）
        let history: Vec<f32> = self.monitor.cpu_history.iter().copied().collect();
        if !history.is_empty() {
            content = content.child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("monitor.cpu_section.load_trend").to_string()),
                    )
                    .child(render_percent_sparkline(
                        &history,
                        44.0,
                        color,
                        p.muted,
                        self.monitor.chart_type,
                    )),
            );
        }

        // 各核心负载分布
        if !stats.cpu_cores.is_empty() {
            content = content.child(
                v_flex()
                    .gap_1()
                    .child(
                        div().text_xs().text_color(p.muted_foreground).child(
                            rust_i18n::t!(
                                "monitor.cpu_section.core_distribution",
                                count = stats.cpu_cores.len()
                            )
                            .to_string(),
                        ),
                    )
                    .child(
                        h_flex()
                            .h(px(32.0))
                            .w_full()
                            .overflow_hidden()
                            .items_end()
                            .gap_0p5()
                            .children(stats.cpu_cores.iter().map(|core| {
                                let fraction = (core.percent / 100.0).clamp(0.0, 1.0);
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px((32.0 * fraction).max(2.0)))
                                    .rounded_t_xs()
                                    .bg(usage_color(core.percent, p))
                            })),
                    ),
            );
        }

        CollapsibleCard::new(
            "card-cpu",
            IconName::Cpu,
            rust_i18n::t!("monitor.cpu").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .summary_always(true)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("cpu", cx);
            }),
        )
    }
}
