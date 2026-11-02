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
use crate::monitor::{SampleIntervalMode, SparklineChartType, usage_bar, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ContainerDetails, ContainerProcess, ContainerStats, TabId};

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
