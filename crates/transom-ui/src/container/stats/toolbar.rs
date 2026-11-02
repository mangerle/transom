//! 统计页工具栏：采样档位、刷新与错误提示。

use super::*;

/// 工具条：采样档位、走势图形态与右侧的采样状态。
pub(crate) fn stats_toolbar(
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
