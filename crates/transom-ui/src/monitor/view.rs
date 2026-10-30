//! 监控抽屉容器视图与组装管道。

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::monitor::types::{SampleIntervalMode, SparklineChartType};
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 渲染右侧监控抽屉。
    pub(crate) fn render_monitor_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let stats = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
            .and_then(|tab| tab.system_stats().cloned());

        let mut body = v_flex().gap_2p5().p_2p5();

        // 1. 异常前哨感知横幅（OOM 告警横幅）
        if let Some(stats_ref) = &stats
            && let Some(oom) = &stats_ref.recent_oom
        {
            body = body.child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(p.danger.opacity(0.15))
                    .border_1()
                    .border_color(p.danger.opacity(0.4))
                    .items_center()
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
                                    .child(rust_i18n::t!("monitor.view.oom_title").to_string()),
                            )
                            .child(
                                div().text_xs().text_color(p.foreground).child(
                                    rust_i18n::t!(
                                        "monitor.view.oom_killed",
                                        name = &oom.process_name,
                                        pid = oom.pid
                                    )
                                    .to_string(),
                                ),
                            ),
                    ),
            );
        }

        // 2. 各板块按需组装
        if let Some(stats) = &stats {
            body = body
                .child(self.render_cpu_section_card(stats, cx, p))
                .child(self.render_memory_section_card(stats, cx, p))
                .child(self.render_disk_section_card(stats, cx, p))
                .child(self.render_network_section_card(stats, cx, p))
                .child(self.render_ports_section_card(stats, cx, p, "drawer"))
                .child(self.render_services_section_card(stats, cx, p));
        } else {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.waiting").to_string()),
            );
        }

        // 3. 进程管理板块
        body = body.child(self.render_process_section_card(cx, p, "drawer"));

        // 抽屉容器布局：尺寸由外部 resizable_panel 统一约束，内部填满并自适应。
        let drawer_width = self.monitor.drawer_width.clamp(360.0, 750.0);
        let mut panel = v_flex()
            .id("monitor-panel")
            .size_full()
            .overflow_hidden()
            .bg(p.background);

        // 顶栏工具条：标题、采样档位芯片、刷新、关闭
        let current_interval = self.monitor.interval_mode;
        let header = h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            .w_full()
            .overflow_hidden()
            .justify_between()
            .items_center()
            .px_3()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .flex_shrink_1()
                    .child(Icon::new(IconName::Gauge).small().text_color(p.primary))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(rust_i18n::t!("monitor.title").to_string()),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .flex_shrink_0()
                    // 采样频率档位切换
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(
                                Button::new("sample-rate-fast")
                                    .xsmall()
                                    .label(SampleIntervalMode::Fast.label())
                                    .selected(current_interval == SampleIntervalMode::Fast)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_monitor_interval_mode(
                                            SampleIntervalMode::Fast,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new("sample-rate-normal")
                                    .xsmall()
                                    .label(SampleIntervalMode::Normal.label())
                                    .selected(current_interval == SampleIntervalMode::Normal)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_monitor_interval_mode(
                                            SampleIntervalMode::Normal,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new("sample-rate-eco")
                                    .xsmall()
                                    .label(SampleIntervalMode::Eco.label())
                                    .selected(current_interval == SampleIntervalMode::Eco)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_monitor_interval_mode(SampleIntervalMode::Eco, cx);
                                    })),
                            ),
                    )
                    // 走势图形态切换（折线面积图 / 柱状图）
                    .child(
                        Button::new("monitor-toggle-chart-type")
                            .ghost()
                            .xsmall()
                            .icon(match self.monitor.chart_type {
                                SparklineChartType::Line => IconName::ChartLine,
                                SparklineChartType::Bar => IconName::ChartBar,
                            })
                            .tooltip(match self.monitor.chart_type {
                                SparklineChartType::Line => {
                                    rust_i18n::t!("monitor.view.chart_line_tooltip").to_string()
                                }
                                SparklineChartType::Bar => {
                                    rust_i18n::t!("monitor.view.chart_bar_tooltip").to_string()
                                }
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.monitor.chart_type = match this.monitor.chart_type {
                                    SparklineChartType::Line => SparklineChartType::Bar,
                                    SparklineChartType::Bar => SparklineChartType::Line,
                                };
                                cx.notify();
                            })),
                    )
                    // 刷新按钮
                    .child(
                        Button::new("monitor-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sample_processes(cx);
                            })),
                    )
                    // 宽度缩放切换芯片（快速切换 420px / 600px）
                    .child(
                        Button::new("monitor-toggle-width")
                            .ghost()
                            .xsmall()
                            .icon(if drawer_width > 480.0 {
                                IconName::Minimize2
                            } else {
                                IconName::Maximize2
                            })
                            .tooltip(if drawer_width > 480.0 {
                                rust_i18n::t!("monitor.view.width_compact_tooltip").to_string()
                            } else {
                                rust_i18n::t!("monitor.view.width_wide_tooltip").to_string()
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_monitor_drawer_width(window, cx);
                            })),
                    )
                    // 脱离为独立标签页按钮
                    .child(
                        Button::new("monitor-maximize-to-tab")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ArrowUpRight)
                            .tooltip(rust_i18n::t!("monitor.view.detach_tooltip").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.maximize_monitor_to_tab(window, cx);
                            })),
                    )
                    // 关闭抽屉按钮
                    .child(
                        Button::new("monitor-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip(rust_i18n::t!("monitor.view.close_tooltip").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.monitor.open = false;
                                this.monitor.processes.clear();
                                cx.notify();
                            })),
                    ),
            );

        panel = panel.child(header);

        // 错误提示条
        if let Some(error) = &self.monitor.error {
            panel = panel.child(
                div()
                    .px_3()
                    .py_1p5()
                    .bg(p.danger.opacity(0.1))
                    .text_xs()
                    .text_color(p.danger)
                    .child(error.clone()),
            );
        }

        // 可滚动卡片内容区
        panel
            .child(
                div()
                    .id("monitor-body")
                    .w_full()
                    .flex_1()
                    .overflow_hidden()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    /// 渲染全屏独立的主机系统监控看板标签页内容。
    pub(crate) fn render_monitor_tab_content(
        &mut self,
        tab_id: TabId,
        target_tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let target_tab = self.workspace.tab(target_tab_id);
        let target_title = target_tab
            .map(|t| t.display_name())
            .unwrap_or_else(|| rust_i18n::t!("monitor.view.host_fallback").to_string());
        let stats = target_tab.and_then(|t| t.system_stats().cloned());

        let current_interval = self.monitor.interval_mode;

        // 顶栏宽幅工具条
        let mut header = h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            .w_full()
            .justify_between()
            .items_center()
            .px_4()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .bg(p.background);

        // 顶栏左侧：图标与标题、主机概要
        let mut title_part = h_flex()
            .gap_2p5()
            .items_center()
            .child(Icon::new(IconName::Gauge).text_color(p.primary))
            .child(
                h_flex().gap_2().items_baseline().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.foreground)
                        .child(
                            rust_i18n::t!("monitor.view.tab_header", host = &target_title)
                                .to_string(),
                        ),
                ),
            );

        if let Some(stats_ref) = &stats {
            let cores = stats_ref.cpu_cores.len();
            let mem_gb = stats_ref.mem_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let days = stats_ref.uptime_secs / 86400;
            let hours = (stats_ref.uptime_secs % 86400) / 3600;
            let uptime_str = if days > 0 {
                rust_i18n::t!("monitor.view.uptime_days", days = days, hours = hours).to_string()
            } else {
                let mins = (stats_ref.uptime_secs % 3600) / 60;
                rust_i18n::t!("monitor.view.uptime_hours", hours = hours, minutes = mins)
                    .to_string()
            };

            let mut info_flex = h_flex().gap_1p5().text_xs().text_color(p.muted_foreground);
            if cores > 0 {
                info_flex = info_flex
                    .child("•")
                    .child(rust_i18n::t!("monitor.view.cpu_cores", count = cores).to_string());
            }
            if stats_ref.mem_total_bytes > 0 {
                let mem_gb_text = format!("{mem_gb:.1}");
                info_flex = info_flex
                    .child("•")
                    .child(rust_i18n::t!("monitor.view.mem_total", gb = mem_gb_text).to_string());
            }
            if stats_ref.uptime_secs > 0 {
                info_flex = info_flex.child("•").child(uptime_str);
            }
            title_part = title_part.child(info_flex);
        }

        header = header.child(title_part);

        // 顶栏右侧：采样频率切换、刷新、停靠为抽屉、关闭
        let actions = h_flex()
            .gap_2()
            .items_center()
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        Button::new("tab-sample-rate-fast")
                            .xsmall()
                            .label(SampleIntervalMode::Fast.label())
                            .selected(current_interval == SampleIntervalMode::Fast)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_monitor_interval_mode(SampleIntervalMode::Fast, cx);
                            })),
                    )
                    .child(
                        Button::new("tab-sample-rate-normal")
                            .xsmall()
                            .label(SampleIntervalMode::Normal.label())
                            .selected(current_interval == SampleIntervalMode::Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_monitor_interval_mode(SampleIntervalMode::Normal, cx);
                            })),
                    )
                    .child(
                        Button::new("tab-sample-rate-eco")
                            .xsmall()
                            .label(SampleIntervalMode::Eco.label())
                            .selected(current_interval == SampleIntervalMode::Eco)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_monitor_interval_mode(SampleIntervalMode::Eco, cx);
                            })),
                    ),
            )
            .child(
                Button::new("tab-monitor-refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip(rust_i18n::t!("monitor.view.refresh_tooltip").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sample_processes(cx);
                    })),
            )
            .child(
                Button::new("tab-monitor-dock-drawer")
                    .ghost()
                    .xsmall()
                    .icon(IconName::PanelRightOpen)
                    .label(rust_i18n::t!("monitor.view.dock_label").to_string())
                    .tooltip(rust_i18n::t!("monitor.view.dock_tooltip").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dock_monitor_to_drawer(tab_id, target_tab_id, window, cx);
                    })),
            )
            .child(
                Button::new("tab-monitor-close-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(rust_i18n::t!("monitor.view.close_tab_tooltip").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_pane(tab_id, window, cx);
                    })),
            );

        header = header.child(actions);

        // OOM 警报 Banner
        let mut oom_banner = None;
        if let Some(stats_ref) = &stats
            && let Some(oom) = &stats_ref.recent_oom
        {
            oom_banner = Some(
                h_flex()
                    .w_full()
                    .gap_2()
                    .p_2p5()
                    .rounded_md()
                    .bg(p.danger.opacity(0.15))
                    .border_1()
                    .border_color(p.danger.opacity(0.4))
                    .items_center()
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
                                    .child(rust_i18n::t!("monitor.view.oom_title").to_string()),
                            )
                            .child(
                                div().text_xs().text_color(p.foreground).child(
                                    rust_i18n::t!(
                                        "monitor.view.oom_killed_full",
                                        name = &oom.process_name,
                                        pid = oom.pid
                                    )
                                    .to_string(),
                                ),
                            ),
                    ),
            );
        }

        // 主体内容
        let content = if let Some(stats) = &stats {
            let left_col = v_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .child(self.render_cpu_section_card(stats, cx, p))
                .child(self.render_memory_section_card(stats, cx, p))
                .child(self.render_disk_section_card(stats, cx, p));

            let right_col = v_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .child(self.render_network_section_card(stats, cx, p))
                .child(self.render_ports_section_card(stats, cx, p, "tab"))
                .child(self.render_services_section_card(stats, cx, p));

            let main_grid = h_flex().w_full().gap_3().child(left_col).child(right_col);
            let process_card = self.render_process_section_card(cx, p, "tab");

            v_flex()
                .w_full()
                .max_w(px(1800.0))
                .mx_auto()
                .gap_3()
                .children(oom_banner)
                .child(main_grid)
                .child(process_card)
        } else {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Spinner::new().color(p.muted_foreground))
                .child(
                    div()
                        .text_sm()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("monitor.waiting").to_string()),
                )
        };

        v_flex()
            .size_full()
            .bg(p.background)
            .child(header)
            .child(
                div()
                    .id("monitor-tab-scroll-viewport")
                    .size_full()
                    .flex_1()
                    .overflow_y_scroll()
                    .p_4()
                    .child(content),
            )
            .into_any_element()
    }
}
