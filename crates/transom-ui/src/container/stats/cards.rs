//! CPU/内存/网络/磁盘四张统计卡片。

use super::*;
use gpui_kit::prelude::FluentBuilder;

/// 统计走势波形图的标准高度，提供更开阔的波形视觉呈现。
pub(crate) const STAT_SPARKLINE_HEIGHT: f32 = 72.0;

pub(crate) fn cpu_card(
    stats: &ContainerStats,
    history: &[f32],
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let color = usage_color(stats.cpu_percent, p);
    let mut content = v_flex().flex_1().min_h_0().gap_2();

    let (cpu_val, cpu_unit) = if chrome.waiting {
        ("—".to_string(), String::new())
    } else {
        (format!("{:.1}", stats.cpu_percent), "%".to_string())
    };

    let hero_row = h_flex()
        .items_baseline()
        .justify_between()
        .child(
            h_flex()
                .gap_2()
                .items_baseline()
                .child(hero_stat(&cpu_val, &cpu_unit, color, p))
                .when(stats.cpu_count > 0 && !chrome.waiting, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.core_count", count = stats.cpu_count).to_string()),
                    )
                }),
        );
    content = content.child(hero_row);

    if !chrome.waiting {
        content = content.child(usage_bar_slim(stats.cpu_percent / 100.0, color, p));
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
                        .h(px(24.0))
                        .w_full()
                        .gap_0p5()
                        .items_end()
                        .children(stats.cpu_cores.iter().map(|percent| {
                            let fraction = (percent / 100.0).clamp(0.0, 1.0);
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px((24.0 * fraction).max(2.0)))
                                .rounded_t_xs()
                                .bg(usage_color(*percent, p))
                        })),
                ),
        );
    }
    let mut display_history = history.to_vec();
    if !chrome.waiting {
        display_history.push(stats.cpu_percent);
    }
    let peak_cpu = display_history
        .iter()
        .copied()
        .fold(0.0f32, f32::max)
        .max(stats.cpu_percent);
    content = content
        .child(trend_label_with_peak(
            chrome,
            if chrome.waiting { None } else { Some(peak_cpu) },
        ))
        .child(render_percent_sparkline(
            &display_history,
            STAT_SPARKLINE_HEIGHT,
            color,
            p.muted,
            chrome.chart,
        ));

    let summary: AnyElement = if let Some((one, five, fifteen)) = stats.host_load {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(format!(
                "{} {one:.2}, {five:.2}, {fifteen:.2}",
                rust_i18n::t!("container.stats.host_load")
            ))
            .into_any_element()
    } else {
        div().into_any_element()
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
    let mut content = v_flex().flex_1().min_h_0().gap_2();

    let (mem_val, mem_unit) = if chrome.waiting {
        ("—".to_string(), String::new())
    } else {
        (format!("{:.1}", stats.memory_percent), "%".to_string())
    };

    let hero_row = h_flex()
        .items_baseline()
        .justify_between()
        .child(
            h_flex()
                .gap_2()
                .items_baseline()
                .child(hero_stat(&mem_val, &mem_unit, color, p))
                .when(!chrome.waiting && stats.memory_limit > 0, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!(
                                "{} / {}",
                                crate::format::format_size(stats.memory_used),
                                crate::format::format_size(stats.memory_limit)
                            )),
                    )
                }),
        );
    content = content.child(hero_row);

    if !chrome.waiting {
        content = content.child(usage_bar_slim(stats.memory_percent / 100.0, color, p));
    }
    let value = |bytes: u64| {
        if chrome.waiting {
            "—".to_string()
        } else {
            crate::format::format_size(bytes)
        }
    };
    let mut display_history = history.to_vec();
    if !chrome.waiting {
        display_history.push(stats.memory_percent);
    }
    let peak_mem = display_history
        .iter()
        .copied()
        .fold(0.0f32, f32::max)
        .max(stats.memory_percent);
    let content = content
        .child(trend_label_with_peak(
            chrome,
            if chrome.waiting { None } else { Some(peak_mem) },
        ))
        .child(render_percent_sparkline(
            &display_history,
            STAT_SPARKLINE_HEIGHT,
            color,
            p.muted,
            chrome.chart,
        ));
    let summary: AnyElement = if chrome.waiting {
        div().into_any_element()
    } else {
        h_flex()
            .flex_1()
            .justify_end()
            .gap_3()
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
                            .font_family(p.mono_font_family.clone())
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
                            .font_family(p.mono_font_family.clone())
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

    let (rx_val, rx_unit) = if chrome.waiting {
        ("—".to_string(), String::new())
    } else {
        split_rate(rx)
    };
    let (tx_val, tx_unit) = if chrome.waiting {
        ("—".to_string(), String::new())
    } else {
        split_rate(tx)
    };

    let hero_row = h_flex()
        .items_baseline()
        .gap_6()
        .child(
            h_flex()
                .gap_1p5()
                .items_baseline()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.primary)
                        .child("↓"),
                )
                .child(hero_stat(&rx_val, &rx_unit, p.primary, p)),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_baseline()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.warning)
                        .child("↑"),
                )
                .child(hero_stat(&tx_val, &tx_unit, p.warning, p)),
        );

    let mut content = v_flex()
        .flex_1()
        .min_h_0()
        .gap_2()
        .child(hero_row)
        .child(trend_label(chrome))
        .child(render_network_dual_sparkline(
            rx_rates,
            tx_rates,
            STAT_SPARKLINE_HEIGHT,
            p.primary,
            p.warning,
            p.muted,
            chrome.chart,
        ));

    if !interfaces.is_empty() {
        content = content.child(
            h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(div().child(rust_i18n::t!("container.stats.interfaces").to_string()))
                .child(div().child("·"))
                .child(
                    div().child(
                        interfaces
                            .iter()
                            .take(3)
                            .map(|(name, _, _)| name.as_str())
                            .collect::<Vec<_>>()
                            .join(" · "),
                    ),
                ),
        );
    }

    stat_card(
        "container-stats-card-network",
        IconName::Network,
        rust_i18n::t!("container.stats.network").to_string(),
        div().into_any_element(),
        content.into_any_element(),
        p,
    )
}

pub(crate) fn disk_card(data: DiskCardData<'_>, chrome: &StatsChrome) -> AnyElement {
    let p = chrome.p;
    let DiskCardData {
        details,
        read_rate,
        write_rate,
        read_rates,
        write_rates,
        devices: _,
    } = data;

    let (r_val, r_unit) = if chrome.waiting {
        ("—".to_string(), String::new())
    } else {
        split_rate(read_rate)
    };
    let (w_val, w_unit) = if chrome.waiting {
        ("—".to_string(), String::new())
    } else {
        split_rate(write_rate)
    };

    let hero_row = h_flex()
        .items_baseline()
        .gap_6()
        .child(
            h_flex()
                .gap_1p5()
                .items_baseline()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.primary)
                        .child("R"),
                )
                .child(hero_stat(&r_val, &r_unit, p.primary, p)),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_baseline()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.warning)
                        .child("W"),
                )
                .child(hero_stat(&w_val, &w_unit, p.warning, p)),
        );

    let mut content = v_flex()
        .flex_1()
        .min_h_0()
        .gap_2()
        .child(hero_row)
        .child(trend_label(chrome))
        .child(render_network_dual_sparkline(
            read_rates,
            write_rates,
            STAT_SPARKLINE_HEIGHT,
            p.primary,
            p.warning,
            p.muted,
            chrome.chart,
        ));

    let rw_size_str = match details {
        Some(details) if details.size_rw > 0 => crate::format::format_size(details.size_rw),
        Some(_) => "—".to_string(),
        None => rust_i18n::t!("container.stats.loading").to_string(),
    };
    let rootfs_str = match details {
        Some(details) if details.size_root_fs > 0 => crate::format::format_size(details.size_root_fs),
        _ => "—".to_string(),
    };
    content = content.child(
        h_flex()
            .gap_4()
            .items_center()
            .text_xs()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.writable_layer").to_string()),
                    )
                    .child(
                        div()
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.foreground)
                            .child(rw_size_str),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.stats.image_size").to_string()),
                    )
                    .child(
                        div()
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.foreground)
                            .child(rootfs_str),
                    ),
            )
            .when(details.is_some_and(|d| !d.mounts.is_empty()), |this| {
                let count = details.map(|d| d.mounts.len()).unwrap_or(0);
                this.child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.stats.mounts").to_string()),
                        )
                        .child(
                            div()
                                .font_family(p.mono_font_family.clone())
                                .text_color(p.foreground)
                                .child(count.to_string()),
                        ),
                )
            }),
    );

    stat_card(
        "container-stats-card-disk",
        IconName::HardDrive,
        rust_i18n::t!("container.stats.disk").to_string(),
        div().into_any_element(),
        content.into_any_element(),
        p,
    )
}
