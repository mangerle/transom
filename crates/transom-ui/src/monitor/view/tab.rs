//! 全屏主机监控标签页内容。

use super::*;

impl AppShell {
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
