//! 主机监控抽屉主面板。

use super::*;

impl MonitorView {
    /// 渲染右侧监控抽屉。
    pub(crate) fn render_monitor_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let stats = self.stats().cloned();

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
        let drawer_width = self.state.drawer_width.clamp(360.0, 750.0);
        let mut panel = v_flex()
            .id("monitor-panel")
            .size_full()
            .overflow_hidden()
            .bg(p.background);

        // 顶栏工具条：标题、采样档位芯片、刷新、关闭
        let current_interval = self.state.interval_mode;
        let actions = h_flex()
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
                                if this.set_interval_mode(SampleIntervalMode::Fast) {
                                    cx.notify();
                                    let interval = this.interval_mode().interval();
                                    cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(
                                        interval,
                                    ));
                                }
                            })),
                    )
                    .child(
                        Button::new("sample-rate-normal")
                            .xsmall()
                            .label(SampleIntervalMode::Normal.label())
                            .selected(current_interval == SampleIntervalMode::Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.set_interval_mode(SampleIntervalMode::Normal) {
                                    cx.notify();
                                    let interval = this.interval_mode().interval();
                                    cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(
                                        interval,
                                    ));
                                }
                            })),
                    )
                    .child(
                        Button::new("sample-rate-eco")
                            .xsmall()
                            .label(SampleIntervalMode::Eco.label())
                            .selected(current_interval == SampleIntervalMode::Eco)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.set_interval_mode(SampleIntervalMode::Eco) {
                                    cx.notify();
                                    let interval = this.interval_mode().interval();
                                    cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(
                                        interval,
                                    ));
                                }
                            })),
                    ),
            )
            // 走势图形态切换（折线面积图 / 柱状图）
            .child(
                Button::new("monitor-toggle-chart-type")
                    .ghost()
                    .xsmall()
                    .icon(match self.state.chart_type {
                        SparklineChartType::Line => IconName::ChartLine,
                        SparklineChartType::Bar => IconName::ChartBar,
                    })
                    .tooltip(match self.state.chart_type {
                        SparklineChartType::Line => {
                            rust_i18n::t!("monitor.view.chart_line_tooltip").to_string()
                        }
                        SparklineChartType::Bar => {
                            rust_i18n::t!("monitor.view.chart_bar_tooltip").to_string()
                        }
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.chart_type = match this.state.chart_type {
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
                        let collapsed = this.state.collapse.processes;
                        cx.emit(crate::monitor::MonitorPanelEvent::SampleProcesses { collapsed });
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
                    .on_click(cx.listener(|this, _, _window, cx| {
                        let new_width = if this.drawer_width() > 480.0 {
                            420.0
                        } else {
                            600.0
                        };
                        this.set_drawer_width(new_width);
                        cx.notify();
                        cx.emit(crate::monitor::MonitorPanelEvent::DrawerWidthChanged(
                            new_width,
                        ));
                    })),
            )
            // 脱离为独立标签页按钮
            .child(
                Button::new("monitor-maximize-to-tab")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowUpRight)
                    .tooltip(rust_i18n::t!("monitor.view.detach_tooltip").to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        // 抽屉开合属于面板状态，先自己落定再回交外壳切标签。
                        this.set_open(false);
                        cx.notify();
                        cx.emit(crate::monitor::MonitorPanelEvent::MaximizeToTab);
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
                        this.state.open = false;
                        this.state.processes.clear();
                        cx.notify();
                    })),
            );
        let header =
            crate::widgets::PanelHeader::new(rust_i18n::t!("monitor.title").to_string(), p)
                .icon(IconName::Gauge)
                .border(p.border)
                .action(actions)
                .render(p);

        panel = panel.child(header);

        // 错误提示条
        if let Some(error) = &self.state.error {
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
}
