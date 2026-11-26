//! 容器统计页：六张卡片（CPU、内存、网络、磁盘、端口映射、进程），两列三行全展开。
//!
//! 设计原理（Why）：主机监控的价值在于「同一屏里同时看到趋势与明细」，容器统计
//! 参照同一套卡片、走势图与采样档位，而不是只有几个进度条。卡片不做折叠：
//! 容器指标本来就少，收起后反而要多点一次才能确认。
//!
//! 代价与局限（Trade-offs）：磁盘与端口来自 inspect 而非 stats，刷新节奏与采样
//! 不同步；容器内没有「宿主 load/swap/systemd 服务」这类宿主级语义，因此没有对应
//! 卡片。

use crate::container::ContainerWorkspace;
use crate::monitor::{MonitorPalette, SampleIntervalMode, SparklineChartType};
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_core::{ContainerDetails, ContainerProcess, ContainerStats};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

mod cards;
mod ports;
mod processes;
mod rates;
mod sparkline;
mod toolbar;

pub(crate) use cards::*;
pub(crate) use ports::*;
pub(crate) use processes::*;
pub(crate) use rates::*;
pub(crate) use sparkline::{render_network_dual_sparkline, render_percent_sparkline};
pub(crate) use toolbar::*;

/// 进程表最多展示的行数：再多会把统计页拖成一张进程管理器。
const PROCESS_LIMIT: usize = 60;

pub(crate) struct ContainerStatsView<'a> {
    pub(crate) stats: Option<&'a ContainerStats>,
    pub(crate) history: &'a [ContainerStats],
    pub(crate) chart: SparklineChartType,
    pub(crate) interval: SampleIntervalMode,
    pub(crate) error: Option<&'a str>,
    pub(crate) details: Option<&'a ContainerDetails>,
    pub(crate) browser_host: &'a str,
    pub(crate) processes: &'a [ContainerProcess],
    pub(crate) processes_loading: bool,
    pub(crate) processes_error: Option<&'a str>,
    pub(crate) is_running: bool,
    /// 是否已选中容器：没有选中时统计页只给提示，不画空卡片。
    pub(crate) has_selection: bool,
    pub(crate) ports_collapsed: bool,
    pub(crate) processes_collapsed: bool,
}

struct StatsSeries {
    cpu_history: Vec<f32>,
    memory_history: Vec<f32>,
    rx_rates: Vec<u64>,
    tx_rates: Vec<u64>,
    read_rates: Vec<u64>,
    write_rates: Vec<u64>,
    interface_rates: Vec<(String, u64, u64)>,
    current_rx: u64,
    current_tx: u64,
    current_read: u64,
    current_write: u64,
}

impl StatsSeries {
    fn build(history: &[ContainerStats], interval: SampleIntervalMode) -> Self {
        let seconds = interval.interval().as_secs_f32().max(0.5);
        let rx_rates = per_second(history, |sample| sample.network_rx, seconds);
        let tx_rates = per_second(history, |sample| sample.network_tx, seconds);
        let read_rates = per_second(history, |sample| sample.block_read, seconds);
        let write_rates = per_second(history, |sample| sample.block_write, seconds);
        Self {
            cpu_history: history.iter().map(|sample| sample.cpu_percent).collect(),
            memory_history: history.iter().map(|sample| sample.memory_percent).collect(),
            current_rx: rx_rates.last().copied().unwrap_or(0),
            current_tx: tx_rates.last().copied().unwrap_or(0),
            current_read: read_rates.last().copied().unwrap_or(0),
            current_write: write_rates.last().copied().unwrap_or(0),
            rx_rates,
            tx_rates,
            read_rates,
            write_rates,
            interface_rates: latest_interface_rates(history, seconds),
        }
    }
}

struct StatsGridParams<'a> {
    view: &'a Entity<ContainerWorkspace>,
    stats: &'a ContainerStats,
    details: Option<&'a ContainerDetails>,
    browser_host: &'a str,
    processes: &'a [ContainerProcess],
    processes_loading: bool,
    processes_error: Option<&'a str>,
    is_running: bool,
    ports_collapsed: bool,
    processes_collapsed: bool,
    series: &'a StatsSeries,
    chrome: &'a StatsChrome<'a>,
}

pub(crate) fn render_container_stats(
    view: &Entity<ContainerWorkspace>,
    params: ContainerStatsView<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerStatsView {
        stats,
        history,
        chart,
        interval,
        error,
        details,
        browser_host,
        processes,
        processes_loading,
        processes_error,
        is_running,
        has_selection,
        ports_collapsed,
        processes_collapsed,
    } = params;
    let waiting = stats.is_none() && error.is_none();
    let trend = trend_window(interval);
    let chrome = StatsChrome {
        chart,
        trend: &trend,
        p,
        waiting,
    };
    let toolbar = stats_toolbar(view, interval, error, &chrome);
    if !has_selection {
        return render_stats_empty(toolbar, p);
    }
    let placeholder = ContainerStats::default();
    let stats = stats.unwrap_or(&placeholder);
    let series = StatsSeries::build(history, interval);
    let content = render_stats_content(StatsGridParams {
        view,
        stats,
        details,
        browser_host,
        processes,
        processes_loading,
        processes_error,
        is_running,
        ports_collapsed,
        processes_collapsed,
        series: &series,
        chrome: &chrome,
    });
    render_stats_shell(toolbar, content)
}

fn render_stats_shell(toolbar: AnyElement, content: AnyElement) -> AnyElement {
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

fn render_stats_empty(toolbar: AnyElement, p: &Palette) -> AnyElement {
    v_flex()
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
        .into_any_element()
}

fn render_stats_content(params: StatsGridParams<'_>) -> AnyElement {
    let StatsGridParams {
        view,
        stats,
        details,
        browser_host,
        processes,
        processes_loading,
        processes_error,
        is_running,
        ports_collapsed,
        processes_collapsed,
        series,
        chrome,
    } = params;
    let metrics = render_stats_metrics(stats, details, series, chrome);
    let tables = render_stats_tables(
        view,
        StatsTablesParams {
            details,
            browser_host,
            processes,
            processes_loading,
            processes_error,
            is_running,
            ports_collapsed,
            processes_collapsed,
            chrome,
        },
    );
    let mut content = v_flex().gap_3().p_3();
    if let Some(banner) = render_oom_banner(details, chrome.p) {
        content = content.child(banner);
    }
    content.child(metrics).child(tables).into_any_element()
}

fn render_oom_banner(details: Option<&ContainerDetails>, p: &Palette) -> Option<AnyElement> {
    details.is_some_and(|details| details.oom_killed).then(|| {
        h_flex()
            .gap_2()
            .items_center()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.danger.opacity(0.15))
            .border_1()
            .border_color(p.danger.opacity(0.4))
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::WARNING))
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
    })
}

fn render_stats_metrics(
    stats: &ContainerStats,
    details: Option<&ContainerDetails>,
    series: &StatsSeries,
    chrome: &StatsChrome<'_>,
) -> AnyElement {
    let cpu = cpu_card(stats, &series.cpu_history, chrome);
    let memory = memory_card(stats, &series.memory_history, chrome);
    let network = network_card(
        series.current_rx,
        series.current_tx,
        &series.rx_rates,
        &series.tx_rates,
        &series.interface_rates,
        chrome,
    );
    let disk = disk_card(
        DiskCardData {
            details,
            read_rate: series.current_read,
            write_rate: series.current_write,
            read_rates: &series.read_rates,
            write_rates: &series.write_rates,
        },
        chrome,
    );
    v_flex()
        .gap_3()
        .child(h_flex().items_stretch().gap_3().child(cpu).child(memory))
        .child(h_flex().items_stretch().gap_3().child(network).child(disk))
        .into_any_element()
}

struct StatsTablesParams<'a> {
    details: Option<&'a ContainerDetails>,
    browser_host: &'a str,
    processes: &'a [ContainerProcess],
    processes_loading: bool,
    processes_error: Option<&'a str>,
    is_running: bool,
    ports_collapsed: bool,
    processes_collapsed: bool,
    chrome: &'a StatsChrome<'a>,
}

fn render_stats_tables(
    view: &Entity<ContainerWorkspace>,
    params: StatsTablesParams<'_>,
) -> AnyElement {
    let ports = ports_card(
        view,
        params.details,
        params.browser_host,
        params.ports_collapsed,
        params.chrome,
    );
    let processes = processes_card(
        view,
        processes::ContainerProcessesView {
            processes: params.processes,
            loading: params.processes_loading,
            error: params.processes_error,
            is_running: params.is_running,
            collapsed: params.processes_collapsed,
        },
        params.chrome,
    );
    v_flex()
        .gap_3()
        .child(ports)
        .child(processes)
        .into_any_element()
}
