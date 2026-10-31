//! 容器统计页：六张卡片（CPU、内存、网络、磁盘、端口映射、进程），两列三行全展开。
//!
//! 设计原理（Why）：主机监控的价值在于「同一屏里同时看到趋势与明细」，容器统计
//! 参照同一套卡片、走势图与采样档位，而不是只有几个进度条。卡片不做折叠：
//! 容器指标本来就少，收起后反而要多点一次才能确认。
//!
//! 代价与局限（Trade-offs）：磁盘与端口来自 inspect 而非 stats，刷新节奏与采样
//! 不同步；容器内没有「宿主 load/swap/systemd 服务」这类宿主级语义，因此没有对应
//! 卡片。

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::monitor::{
    SampleIntervalMode, SparklineChartType, render_network_dual_sparkline,
    render_percent_sparkline, usage_bar, usage_color,
};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ContainerDetails, ContainerProcess, ContainerStats, TabId};

/// 进程表最多展示的行数：再多会把统计页拖成一张进程管理器。
const PROCESS_LIMIT: usize = 60;

pub(crate) struct ContainerStatsView<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) stats: Option<&'a ContainerStats>,
    pub(crate) history: &'a [ContainerStats],
    pub(crate) chart: SparklineChartType,
    pub(crate) interval: SampleIntervalMode,
    pub(crate) error: Option<&'a str>,
    pub(crate) details: Option<&'a ContainerDetails>,
    pub(crate) processes: &'a [ContainerProcess],
    pub(crate) processes_loading: bool,
    pub(crate) processes_error: Option<&'a str>,
    /// 是否已选中容器：没有选中时统计页只给提示，不画空卡片。
    pub(crate) has_selection: bool,
}

pub(crate) fn render_container_stats(
    view: ContainerStatsView<'_>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let ContainerStatsView {
        tab_id,
        stats,
        history,
        chart,
        interval,
        error,
        details,
        processes,
        processes_loading,
        processes_error,
        has_selection,
    } = view;

    let waiting = stats.is_none() && error.is_none();
    let toolbar = stats_toolbar(tab_id, interval, chart, waiting, error, p, cx);
    if !has_selection {
        return v_flex()
            .size_full()
            .child(toolbar)
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.stats.select_first").to_string()),
            )
            .into_any_element();
    }
    // 没数据也先把六张卡片摆出来，用「—」占位；等待/失败只在工具条右侧提示，
    // 不再让整页只剩一句「正在采样统计…」。
    let placeholder = ContainerStats::default();
    let stats = stats.unwrap_or(&placeholder);
    let seconds = interval.interval().as_secs_f32().max(0.5);
    let cpu_history: Vec<f32> = history.iter().map(|sample| sample.cpu_percent).collect();
    let memory_history: Vec<f32> = history.iter().map(|sample| sample.memory_percent).collect();
    let rx_rates = per_second(history, |sample| sample.network_rx, seconds);
    let tx_rates = per_second(history, |sample| sample.network_tx, seconds);
    let read_rates = per_second(history, |sample| sample.block_read, seconds);
    let write_rates = per_second(history, |sample| sample.block_write, seconds);
    let interface_rates = latest_interface_rates(history, seconds);
    let device_rates = latest_device_rates(history, seconds);
    let current_rx = rx_rates.last().copied().unwrap_or(0);
    let current_tx = tx_rates.last().copied().unwrap_or(0);
    let current_read = read_rates.last().copied().unwrap_or(0);
    let current_write = write_rates.last().copied().unwrap_or(0);

    let oom_banner = details.is_some_and(|details| details.oom_killed).then(|| {
        h_flex()
            .gap_2()
            .items_center()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.danger.opacity(0.15))
            .border_1()
            .border_color(p.danger.opacity(0.4))
            .child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(p.danger),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.danger)
                            .child(rust_i18n::t!("container.stats.oom_title").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .child(rust_i18n::t!("container.stats.oom_detail").to_string()),
                    ),
            )
            .into_any_element()
    });

    let trend = trend_window(interval);
    let chrome = StatsChrome {
        chart,
        trend: &trend,
        p,
        waiting,
    };

    let cpu_card = cpu_card(stats, &cpu_history, &chrome);
    let memory_card = memory_card(stats, &memory_history, &chrome);
    let network_card = network_card(
        current_rx,
        current_tx,
        &rx_rates,
        &tx_rates,
        &interface_rates,
        &chrome,
    );
    let disk_card = disk_card(
        DiskCardData {
            details,
            read_rate: current_read,
            write_rate: current_write,
            read_rates: &read_rates,
            write_rates: &write_rates,
            devices: &device_rates,
        },
        &chrome,
    );
    let ports_card = ports_card(details, &chrome);
    let processes_card = processes_card(
        tab_id,
        processes,
        processes_loading,
        processes_error,
        &chrome,
        cx,
    );

    // 三行两列：同一行的高度由较高的卡片决定，左右两列底部因此对齐，
    // 不会再出现左列早早结束、右列拖很长的情况。
    let row_one = h_flex()
        .items_stretch()
        .gap_2()
        .child(cpu_card)
        .child(memory_card);
    let row_two = h_flex()
        .items_stretch()
        .gap_2()
        .child(network_card)
        .child(disk_card);
    let row_three = h_flex()
        .items_stretch()
        .gap_2()
        .child(ports_card)
        .child(processes_card);
    let grid = v_flex()
        .gap_2()
        .child(row_one)
        .child(row_two)
        .child(row_three);

    let mut content = v_flex().gap_2().p_3();
    if let Some(banner) = oom_banner {
        content = content.child(banner);
    }
    content = content.child(grid);

    v_flex()
        .size_full()
        .child(toolbar)
        .child(
            div()
                .id("container-stats-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(content),
        )
        .into_any_element()
}

/// 工具条：采样档位、走势图形态与右侧的采样状态。
fn stats_toolbar(
    tab_id: TabId,
    interval: SampleIntervalMode,
    chart: SparklineChartType,
    waiting: bool,
    error: Option<&str>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut interval_chips = h_flex().gap_0p5().items_center();
    for mode in [
        SampleIntervalMode::Fast,
        SampleIntervalMode::Normal,
        SampleIntervalMode::Eco,
    ] {
        interval_chips = interval_chips.child(
            Button::new(ElementId::Name(
                format!("container-stats-interval-{:?}", mode).into(),
            ))
            .xsmall()
            .label(mode.label())
            .selected(interval == mode)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_container_stats_interval(tab_id, mode, cx);
            })),
        );
    }
    // 采样状态放在工具条右端：不再单独占一行，也不会把卡片往下推。
    let status: Option<AnyElement> = if let Some(error) = error {
        Some(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Icon::new(IconName::CircleAlert)
                        .xsmall()
                        .text_color(p.danger),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.foreground)
                        .child(error.to_string()),
                )
                .child(
                    Button::new("container-stats-retry")
                        .ghost()
                        .xsmall()
                        .icon(IconName::RefreshCw)
                        .label(rust_i18n::t!("common.retry").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ensure_container_stats(tab_id, cx);
                            this.ensure_container_processes(tab_id);
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    } else if waiting {
        Some(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Icon::new(IconName::Loader)
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.waiting").to_string()),
                )
                .into_any_element(),
        )
    } else {
        None
    };
    h_flex()
        .gap_2()
        .p_2()
        .items_center()
        .child(interval_chips)
        .child(div().w(px(1.0)).h(px(14.0)).bg(p.border))
        .child(
            Button::new("container-stats-chart")
                .ghost()
                .xsmall()
                .icon(match chart {
                    SparklineChartType::Line => IconName::ChartLine,
                    SparklineChartType::Bar => IconName::ChartBar,
                })
                .tooltip(
                    rust_i18n::t!(match chart {
                        SparklineChartType::Line => "monitor.view.chart_line_tooltip",
                        SparklineChartType::Bar => "monitor.view.chart_bar_tooltip",
                    })
                    .to_string(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_container_stats_chart(tab_id, cx);
                })),
        )
        .child(div().flex_1())
        .children(status)
        .into_any_element()
}

fn cpu_card(stats: &ContainerStats, history: &[f32], chrome: &StatsChrome) -> AnyElement {
    let p = chrome.p;
    let color = usage_color(stats.cpu_percent, p);
    let mut content = v_flex().gap_2();
    if !chrome.waiting {
        content = content.child(usage_bar(stats.cpu_percent / 100.0, color, p));
    }
    if let Some((one, five, fifteen)) = stats.host_load {
        content = content.child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(
                    div()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.host_load").to_string()),
                )
                .child(
                    div()
                        .text_color(p.foreground)
                        .child(format!("{one:.2}, {five:.2}, {fifteen:.2}")),
                ),
        );
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
    let summary = if chrome.waiting {
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
    stat_card(
        "container-stats-card-cpu",
        IconName::Cpu,
        rust_i18n::t!("container.stats.cpu").to_string(),
        summary,
        content.into_any_element(),
        p,
    )
}

fn memory_card(stats: &ContainerStats, history: &[f32], chrome: &StatsChrome) -> AnyElement {
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
        .child(
            h_flex()
                .justify_between()
                .text_xs()
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
                .justify_between()
                .text_xs()
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
        .child(trend_label(chrome))
        .child(render_percent_sparkline(
            history,
            44.0,
            color,
            p.muted,
            chrome.chart,
        ));
    stat_card(
        "container-stats-card-memory",
        IconName::Server,
        rust_i18n::t!("container.stats.memory").to_string(),
        if chrome.waiting {
            summary_text("—".to_string(), p.muted_foreground)
        } else {
            summary_text(
                format!(
                    "{} / {}",
                    crate::format::format_size(stats.memory_used),
                    crate::format::format_size(stats.memory_limit)
                ),
                color,
            )
        },
        content.into_any_element(),
        p,
    )
}

fn network_card(
    rx: u64,
    tx: u64,
    rx_rates: &[u64],
    tx_rates: &[u64],
    interfaces: &[(String, u64, u64)],
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let mut content =
        v_flex()
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
    if !interfaces.is_empty() {
        content = content.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.interfaces").to_string()),
                )
                .children(interfaces.iter().map(|(name, rx, tx)| {
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .child(div().text_color(p.foreground).child(name.clone()))
                        .child(div().text_color(p.muted_foreground).child(format!(
                            "↓ {}/s  ↑ {}/s",
                            crate::format::format_size(*rx),
                            crate::format::format_size(*tx)
                        )))
                })),
        );
    }
    stat_card(
        "container-stats-card-network",
        IconName::Network,
        rust_i18n::t!("container.stats.network").to_string(),
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
struct StatsChrome<'a> {
    chart: SparklineChartType,
    trend: &'a str,
    p: &'a Palette,
    /// 还没有第一帧数据：数值显示「—」，避免把占位的 0 当成真实值。
    waiting: bool,
}

/// 历史窗口文案：与采样档位对应（40 个槽位）。
fn trend_window(interval: SampleIntervalMode) -> String {
    match interval {
        SampleIntervalMode::Fast => rust_i18n::t!("container.stats.window_40s").to_string(),
        SampleIntervalMode::Normal => rust_i18n::t!("container.stats.window_2m").to_string(),
        SampleIntervalMode::Eco => rust_i18n::t!("container.stats.window_7m").to_string(),
    }
}

fn trend_label(chrome: &StatsChrome) -> AnyElement {
    div()
        .text_xs()
        .text_color(chrome.p.muted_foreground)
        .child(rust_i18n::t!("container.stats.trend", window = chrome.trend).to_string())
        .into_any_element()
}

/// 磁盘卡片数据：字段多且都是借用，收敛为结构体避免长参数列表。
struct DiskCardData<'a> {
    details: Option<&'a ContainerDetails>,
    read_rate: u64,
    write_rate: u64,
    read_rates: &'a [u64],
    write_rates: &'a [u64],
    devices: &'a [(String, u64, u64)],
}

fn disk_card(data: DiskCardData<'_>, chrome: &StatsChrome) -> AnyElement {
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

fn ports_card(details: Option<&ContainerDetails>, chrome: &StatsChrome) -> AnyElement {
    let p = chrome.p;
    let Some(details) = details else {
        return stat_card(
            "container-stats-card-ports",
            IconName::Plug,
            rust_i18n::t!("container.stats.ports").to_string(),
            summary_text("—".to_string(), p.muted_foreground),
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.loading").to_string())
                .into_any_element(),
            p,
        );
    };
    let mut rows = v_flex().gap_1();
    if details.ports.is_empty() && details.exposed_ports.is_empty() {
        rows = rows.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.no_ports").to_string()),
        );
    }
    if !details.ports.is_empty() {
        rows = rows.child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.published").to_string()),
                )
                .children(details.ports.iter().map(|port| {
                    let mapping = match port.public_port {
                        Some(public) => {
                            format!("{public} -> {}/{}", port.private_port, port.protocol)
                        }
                        None => rust_i18n::t!(
                            "container.stats.unpublished_port",
                            port = format!("{}/{}", port.private_port, port.protocol)
                        )
                        .to_string(),
                    };
                    div()
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .text_color(p.foreground)
                        .child(mapping)
                })),
        );
    }
    if !details.exposed_ports.is_empty() {
        rows = rows.child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.exposed").to_string()),
                )
                .children(details.exposed_ports.iter().map(|port| {
                    div()
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .text_color(p.muted_foreground)
                        .child(port.clone())
                })),
        );
    }
    stat_card(
        "container-stats-card-ports",
        IconName::Plug,
        rust_i18n::t!("container.stats.ports").to_string(),
        summary_text(
            rust_i18n::t!(
                "container.stats.ports_summary",
                published = details.ports.len(),
                exposed = details.exposed_ports.len()
            )
            .to_string(),
            p.muted_foreground,
        ),
        rows.into_any_element(),
        p,
    )
}

fn processes_card(
    tab_id: TabId,
    processes: &[ContainerProcess],
    loading: bool,
    error: Option<&str>,
    chrome: &StatsChrome,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let p = chrome.p;
    let summary = h_flex()
        .gap_1p5()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(if loading {
                    rust_i18n::t!("container.stats.process_loading").to_string()
                } else {
                    processes.len().to_string()
                }),
        )
        .child(
            Button::new("container-processes-refresh")
                .ghost()
                .xsmall()
                .icon(IconName::RefreshCw)
                .tooltip(rust_i18n::t!("container.stats.process_refresh").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ensure_container_processes(tab_id);
                    cx.notify();
                })),
        )
        .into_any_element();
    let body: AnyElement = if let Some(error) = error {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(error.to_string())
            .into_any_element()
    } else if processes.is_empty() {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.stats.process_empty").to_string())
            .into_any_element()
    } else {
        let mut sorted: Vec<&ContainerProcess> = processes.iter().collect();
        sorted.sort_by(|left, right| {
            right
                .cpu_percent
                .partial_cmp(&left.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut rows = v_flex().gap_0p5().child(process_header(p)).children(
            sorted
                .iter()
                .take(PROCESS_LIMIT)
                .map(|process| process_row(process, p)),
        );
        if sorted.len() > PROCESS_LIMIT {
            rows = rows.child(div().text_xs().text_color(p.muted_foreground).child(
                rust_i18n::t!("container.stats.process_limit", count = PROCESS_LIMIT).to_string(),
            ));
        }
        rows.into_any_element()
    };
    stat_card(
        "container-stats-card-processes",
        IconName::ListTree,
        rust_i18n::t!("container.stats.processes").to_string(),
        summary,
        body,
        p,
    )
}

fn process_header(p: &Palette) -> AnyElement {
    let label = |text: String, width: f32, right: bool| {
        let mut element = div()
            .w(px(width))
            .flex_none()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(text);
        if right {
            element = element.text_right();
        }
        element
    };
    h_flex()
        .gap_2()
        .items_center()
        .border_color(p.divider(p.border))
        .border_b_1()
        .pb_1()
        .child(label("PID".to_string(), 52.0, false))
        .child(label(
            rust_i18n::t!("container.stats.process_user").to_string(),
            72.0,
            false,
        ))
        .child(label("CPU".to_string(), 52.0, true))
        .child(label(
            rust_i18n::t!("container.stats.process_mem").to_string(),
            52.0,
            true,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.process_command").to_string()),
        )
        .into_any_element()
}

fn process_row(process: &ContainerProcess, p: &Palette) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .text_xs()
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .font_family(p.mono_font_family.clone())
                .text_color(p.muted_foreground)
                .child(process.pid.clone()),
        )
        .child(
            div()
                .w(px(72.0))
                .flex_none()
                .truncate()
                .text_color(p.foreground)
                .child(process.user.clone()),
        )
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .text_right()
                .text_color(usage_color(process.cpu_percent, p))
                .child(format!("{:.1}%", process.cpu_percent)),
        )
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .text_right()
                .text_color(p.muted_foreground)
                .child(format!("{:.1}%", process.mem_percent)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(p.mono_font_family.clone())
                .text_color(p.foreground)
                .child(process.command.clone()),
        )
        .into_any_element()
}

fn stat_card(
    id: &'static str,
    icon: IconName,
    title: String,
    summary: AnyElement,
    content: AnyElement,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .id(id)
        .flex_1()
        .min_w_0()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.12))
        .border_1()
        .border_color(p.divider(p.border))
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_between()
                .px_2p5()
                .py_2()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .min_w_0()
                        .child(Icon::new(icon).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(summary),
        )
        .child(v_flex().gap_2p5().p_2p5().child(content))
        .into_any_element()
}

fn summary_text(value: String, color: Hsla) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(value)
        .into_any_element()
}

/// 相邻采样的增量按秒折算：主机监控的速率口径是 `/s`，容器这里保持一致。
fn per_second(
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

fn latest_interface_rates(history: &[ContainerStats], seconds: f32) -> Vec<(String, u64, u64)> {
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

fn latest_device_rates(history: &[ContainerStats], seconds: f32) -> Vec<(String, u64, u64)> {
    let Some(last) = history.last() else {
        return Vec::new();
    };
    let previous = history.len().checked_sub(2).map(|index| &history[index]);
    last.block_devices
        .iter()
        .map(|device| {
            let (read, write) = previous
                .and_then(|sample| {
                    sample
                        .block_devices
                        .iter()
                        .find(|item| item.device == device.device)
                })
                .map(|item| {
                    (
                        device.read_bytes.saturating_sub(item.read_bytes),
                        device.write_bytes.saturating_sub(item.write_bytes),
                    )
                })
                .unwrap_or((0, 0));
            (
                device.device.clone(),
                (read as f64 / seconds as f64) as u64,
                (write as f64 / seconds as f64) as u64,
            )
        })
        .collect()
}
