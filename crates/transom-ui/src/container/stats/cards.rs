//! CPU/内存/网络/磁盘四张统计卡片。

use super::*;

pub(crate) fn cpu_card(
    stats: &ContainerStats,
    history: &[f32],
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let color = usage_color(stats.cpu_percent, p);
    let mut content = v_flex().gap_2();
    if !chrome.waiting {
        content = content.child(usage_bar(stats.cpu_percent / 100.0, color, p));
    }
    if !stats.cpu_cores.is_empty() {
        content = content.child(
            v_flex()
                .gap_1()
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!("container.stats.core_usage", count = stats.cpu_cores.len())
                            .to_string(),
                    ),
                )
                .child(
                    h_flex()
                        .h(px(32.0))
                        .w_full()
                        .gap_0p5()
                        .items_end()
                        .children(stats.cpu_cores.iter().map(|percent| {
                            let fraction = (percent / 100.0).clamp(0.0, 1.0);
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px((32.0 * fraction).max(2.0)))
                                .rounded_t_xs()
                                .bg(usage_color(*percent, p))
                        })),
                ),
        );
    }
    content = content
        .child(trend_label(chrome))
        .child(render_percent_sparkline(
            history,
            44.0,
            color,
            p.muted,
            chrome.chart,
        ));
    let cpu_summary = if chrome.waiting {
        summary_text("—".to_string(), p.muted_foreground)
    } else if stats.cpu_count > 0 {
        summary_text(
            format!(
                "{:.1}% · {}",
                stats.cpu_percent,
                rust_i18n::t!("container.stats.core_count", count = stats.cpu_count)
            ),
            color,
        )
    } else {
        summary_text(format!("{:.1}%", stats.cpu_percent), color)
    };
    let summary: AnyElement = if let Some((one, five, fifteen)) = stats.host_load {
        h_flex()
            .flex_1()
            .justify_end()
            .gap_4()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!(
                        "{} {one:.2}, {five:.2}, {fifteen:.2}",
                        rust_i18n::t!("container.stats.host_load")
                    )),
            )
            .child(cpu_summary)
            .into_any_element()
    } else {
        cpu_summary
    };
    stat_card(
        "container-stats-card-cpu",
        IconName::Cpu,
        rust_i18n::t!("container.stats.cpu").to_string(),
        summary,
        content.into_any_element(),
        p,
    )
}

pub(crate) fn memory_card(
    stats: &ContainerStats,
    history: &[f32],
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let color = usage_color(stats.memory_percent, p);
    let mut content = v_flex().gap_2();
    if !chrome.waiting {
        content = content.child(usage_bar(stats.memory_percent / 100.0, color, p));
    }
    let value = |bytes: u64| {
        if chrome.waiting {
            "—".to_string()
        } else {
            crate::format::format_size(bytes)
        }
    };
    let content = content
        .child(trend_label(chrome))
        .child(render_percent_sparkline(
            history,
            44.0,
            color,
            p.muted,
            chrome.chart,
        ));
    let summary: AnyElement = if chrome.waiting {
        summary_text("—".to_string(), p.muted_foreground)
    } else {
        h_flex()
            .flex_1()
            .justify_end()
            .gap_4()
            .text_xs()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.rss").to_string()),
                    )
                    .child(
                        div()
                            .text_color(p.foreground)
                            .child(value(stats.memory_rss)),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.cache").to_string()),
                    )
                    .child(
                        div()
                            .text_color(p.foreground)
                            .child(value(stats.memory_cache)),
                    ),
            )
            .into_any_element()
    };
    stat_card(
        "container-stats-card-memory",
        IconName::Server,
        rust_i18n::t!("container.stats.memory").to_string(),
        summary,
        content.into_any_element(),
        p,
    )
}

pub(crate) fn network_card(
    rx: u64,
    tx: u64,
    rx_rates: &[u64],
    tx_rates: &[u64],
    interfaces: &[(String, u64, u64)],
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let content = v_flex()
        .gap_2()
        .child(trend_label(chrome))
        .child(render_network_dual_sparkline(
            rx_rates,
            tx_rates,
            44.0,
            p.primary,
            p.sidebar_accent_foreground,
            p.muted,
            chrome.chart,
        ));
    let mut title = rust_i18n::t!("container.stats.network").to_string();
    if !interfaces.is_empty() {
        let names = interfaces
            .iter()
            .take(2)
            .map(|(name, _, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        title.push_str(" · ");
        title.push_str(&names);
    }
    stat_card(
        "container-stats-card-network",
        IconName::Network,
        title,
        if chrome.waiting {
            summary_text("—".to_string(), p.muted_foreground)
        } else {
            summary_text(
                format!(
                    "↓ {}/s  ↑ {}/s",
                    crate::format::format_size(rx),
                    crate::format::format_size(tx)
                ),
                p.primary,
            )
        },
        content.into_any_element(),
        p,
    )
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
    div()
        .text_xs()
        .text_color(chrome.p.muted_foreground)
        .child(rust_i18n::t!("container.stats.trend", window = chrome.trend).to_string())
        .into_any_element()
}

/// 磁盘卡片数据：字段多且都是借用，收敛为结构体避免长参数列表。
pub(crate) struct DiskCardData<'a> {
    pub(crate) details: Option<&'a ContainerDetails>,
    pub(crate) read_rate: u64,
    pub(crate) write_rate: u64,
    pub(crate) read_rates: &'a [u64],
    pub(crate) write_rates: &'a [u64],
    pub(crate) devices: &'a [(String, u64, u64)],
}

pub(crate) fn disk_card(data: DiskCardData<'_>, chrome: &StatsChrome) -> AnyElement {
    let p = chrome.p;
    let DiskCardData {
        details,
        read_rate,
        write_rate,
        read_rates,
        write_rates,
        devices,
    } = data;
    let mut content = v_flex().gap_2().child(
        h_flex()
            .justify_between()
            .text_xs()
            .child(
                div()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.stats.writable_layer").to_string()),
            )
            .child(div().text_color(p.foreground).child(match details {
                Some(details) if details.size_rw > 0 => crate::format::format_size(details.size_rw),
                Some(_) => "—".to_string(),
                None => rust_i18n::t!("container.stats.loading").to_string(),
            })),
    );
    if let Some(details) = details {
        content = content
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.image_size").to_string()),
                    )
                    .child(
                        div()
                            .text_color(p.foreground)
                            .child(if details.size_root_fs > 0 {
                                crate::format::format_size(details.size_root_fs)
                            } else {
                                "—".to_string()
                            }),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.mounts").to_string()),
                    )
                    .child(
                        div()
                            .text_color(p.foreground)
                            .child(details.mounts.len().to_string()),
                    ),
            );
        if !details.mounts.is_empty() {
            content = content.child(v_flex().gap_0p5().children(
                details.mounts.iter().take(4).map(|mount| {
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(p.muted_foreground)
                        .child(format!("{} -> {}", mount.source, mount.destination))
                }),
            ));
        }
    }
    content = content
        .child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(
                    div()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.block").to_string()),
                )
                .child(div().text_color(p.foreground).child(if chrome.waiting {
                    "—".to_string()
                } else {
                    format!(
                        "{} R/s / {} W/s",
                        crate::format::format_size(read_rate),
                        crate::format::format_size(write_rate)
                    )
                })),
        )
        .child(trend_label(chrome))
        .child(render_network_dual_sparkline(
            read_rates,
            write_rates,
            44.0,
            p.primary,
            p.warning,
            p.muted,
            chrome.chart,
        ));
    if !devices.is_empty() {
        content = content.child(v_flex().gap_0p5().children(devices.iter().take(4).map(
            |(device, read, write)| {
                h_flex()
                    .justify_between()
                    .text_xs()
                    .child(div().text_color(p.muted_foreground).child(device.clone()))
                    .child(div().text_color(p.foreground).child(format!(
                        "{} R/s / {} W/s",
                        crate::format::format_size(*read),
                        crate::format::format_size(*write)
                    )))
            },
        )));
    }
    stat_card(
        "container-stats-card-disk",
        IconName::HardDrive,
        rust_i18n::t!("container.stats.disk").to_string(),
        if chrome.waiting {
            summary_text("—".to_string(), p.muted_foreground)
        } else {
            summary_text(
                format!(
                    "{} R/s / {} W/s",
                    crate::format::format_size(read_rate),
                    crate::format::format_size(write_rate)
                ),
                p.primary,
            )
        },
        content.into_any_element(),
        p,
    )
}
