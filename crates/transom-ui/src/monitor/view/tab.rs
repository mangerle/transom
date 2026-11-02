//! 全屏主机监控标签页内容。

use super::*;

impl MonitorView {
    /// 渲染全屏监控看板。
    ///
    /// 数据由外壳传入而不是在这里反查：目标标签的标题与统计快照属于工作区，
    /// 面板只负责画，避免为了取值而克隆整份会话。
    pub(crate) fn render_monitor_tab_content(
        &mut self,
        params: MonitorTabParams<'_>,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let MonitorTabParams {
            tab_id,
            target_tab_id,
            target_title,
            stats,
        } = params;
        let current_interval = self.state.interval_mode;

        // 顶栏宽幅工具条：左侧标题/概要，右侧采样与窗口动作。
        let header = h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            .w_full()
            .justify_between()
            .items_center()
            .px_4()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .bg(p.background)
            .child(tab_title_part(&target_title, stats, p))
            .child(self.render_tab_actions(current_interval, tab_id, target_tab_id, cx));

        let oom_banner = stats.and_then(|stats| crate::monitor::components::oom_banner(stats, p));
        let content = self.render_tab_body(stats, oom_banner, cx, p);

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

impl MonitorView {
    /// 全屏看板主体：有样本时画双列指标 + 进程卡；无样本时显示等待态。
    fn render_tab_body(
        &mut self,
        stats: Option<&transom_core::SystemStats>,
        oom_banner: Option<AnyElement>,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 主体内容
        let content = if let Some(stats) = stats {
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
        content.into_any_element()
    }
}

/// 全屏监控看板的渲染入参。
///
/// 目标标签的标题与统计快照属于工作区，由外壳读好传入；收成结构体是因为
/// 平铺参数已超过五个，调用点容易把两个 `TabId` 传反。
pub(crate) struct MonitorTabParams<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) target_tab_id: TabId,
    pub(crate) target_title: String,
    /// 目标标签的统计快照只读借用：看板每帧渲染，克隆整份统计没有必要。
    pub(crate) stats: Option<&'a transom_core::SystemStats>,
}

impl MonitorView {
    /// 全屏看板右上角动作：采样档位、刷新、停靠抽屉与关闭。
    /// 单个采样档位按钮：点击后切换档位并请求重启采样。
    fn render_sample_mode_button(
        &mut self,
        mode: crate::monitor::SampleIntervalMode,
        current: crate::monitor::SampleIntervalMode,
        cx: &mut Context<Self>,
    ) -> gpui_kit::component::button::Button {
        let id = match mode {
            crate::monitor::SampleIntervalMode::Fast => "tab-sample-rate-fast",
            crate::monitor::SampleIntervalMode::Normal => "tab-sample-rate-normal",
            crate::monitor::SampleIntervalMode::Eco => "tab-sample-rate-eco",
        };
        Button::new(id)
            .xsmall()
            .label(mode.label())
            .selected(current == mode)
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.set_interval_mode(mode) {
                    cx.notify();
                    let interval = this.interval_mode().interval();
                    cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(interval));
                }
            }))
    }

    fn render_tab_actions(
        &mut self,
        current_interval: crate::monitor::SampleIntervalMode,
        tab_id: transom_core::TabId,
        target_tab_id: transom_core::TabId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 顶栏右侧：采样频率切换、刷新、停靠为抽屉、关闭
        let actions = h_flex()
            .gap_2()
            .items_center()
            .child(
                h_flex()
                    .gap_0p5()
                    .child(self.render_sample_mode_button(
                        SampleIntervalMode::Fast,
                        current_interval,
                        cx,
                    ))
                    .child(self.render_sample_mode_button(
                        SampleIntervalMode::Normal,
                        current_interval,
                        cx,
                    ))
                    .child(self.render_sample_mode_button(
                        SampleIntervalMode::Eco,
                        current_interval,
                        cx,
                    )),
            )
            .child(
                crate::widgets::IconButton::new(
                    "tab-monitor-refresh",
                    IconName::RefreshCw,
                    rust_i18n::t!("monitor.view.refresh_tooltip").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    let collapsed = this.state.collapse.processes;
                    cx.emit(crate::monitor::MonitorPanelEvent::SampleProcesses { collapsed });
                })),
            )
            .child(
                Button::new("tab-monitor-dock-drawer")
                    .ghost()
                    .xsmall()
                    .icon(IconName::PanelRightOpen)
                    .label(rust_i18n::t!("monitor.view.dock_label").to_string())
                    .tooltip(rust_i18n::t!("monitor.view.dock_tooltip").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        // 抽屉开合属于面板状态，先自己落定再回交外壳切标签与关标签。
                        this.set_open(true);
                        cx.notify();
                        cx.emit(crate::monitor::MonitorPanelEvent::DockToDrawer {
                            tab_id,
                            target_tab_id,
                        });
                    })),
            )
            .child(
                crate::widgets::IconButton::new(
                    "tab-monitor-close-btn",
                    IconName::Close,
                    rust_i18n::t!("monitor.view.close_tab_tooltip").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(move |_this, _, _window, cx| {
                    cx.emit(crate::monitor::MonitorPanelEvent::ClosePane { tab_id });
                })),
            );
        actions.into_any_element()
    }
}

/// 全屏看板标题：主机名与核心数/内存/运行时间概要。
fn tab_title_part(
    target_title: &str,
    stats: Option<&transom_core::SystemStats>,
    p: &Palette,
) -> AnyElement {
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
                        rust_i18n::t!("monitor.view.tab_header", host = &target_title).to_string(),
                    ),
            ),
        );

    if let Some(stats_ref) = stats {
        let cores = stats_ref.cpu_cores.len();
        let mem_gb = stats_ref.mem_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let days = stats_ref.uptime_secs / 86400;
        let hours = (stats_ref.uptime_secs % 86400) / 3600;
        let uptime_str = if days > 0 {
            rust_i18n::t!("monitor.view.uptime_days", days = days, hours = hours).to_string()
        } else {
            let mins = (stats_ref.uptime_secs % 3600) / 60;
            rust_i18n::t!("monitor.view.uptime_hours", hours = hours, minutes = mins).to_string()
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

    title_part.into_any_element()
}
