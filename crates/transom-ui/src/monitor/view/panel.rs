//! 主机监控抽屉主面板。

use super::*;
use gpui_kit::prelude::FluentBuilder;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl MonitorView {
    /// 渲染右侧监控抽屉。
    pub(crate) fn render_monitor_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 只读借用：卡片渲染全部接收 &self，克隆整份统计会让每帧多一次
        // 进程/端口/网络列表的深拷贝。
        let stats = self.stats();

        let mut body = v_flex().gap_2p5().p_2p5();

        // 1. 异常前哨感知横幅（OOM 告警横幅）
        if let Some(stats_ref) = stats
            && let Some(banner) = crate::monitor::components::oom_banner(stats_ref, p)
        {
            body = body.child(banner);
        }

        // 2. 各板块按需组装
        if let Some(stats) = stats {
            body = body
                .child(self.render_cpu_section_card(stats, cx, p))
                .child(self.render_memory_section_card(stats, cx, p))
                .child(self.render_disk_section_card(stats, cx, p))
                .child(self.render_network_section_card(stats, cx, p))
                .when(stats.capabilities.linux_diagnostics, |this| {
                    this.child(self.render_ports_section_card(stats, cx, p, "drawer"))
                        .child(self.render_services_section_card(stats, cx, p))
                });
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
        let actions = self.render_header_actions(current_interval, drawer_width, cx);
        let header =
            crate::widgets::PanelHeader::new(rust_i18n::t!("monitor.title").to_string(), p)
                .icon(AppIcon::Domain(DomainIcon::MONITOR))
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

    /// 监控标题栏右侧动作：采样档位、走势形态、刷新、宽度、脱离与关闭。
    ///
    /// 动作区较长且互相独立，抽出来后 `render_monitor_panel` 只负责布局，
    /// 调整某个按钮的 tooltip 或禁用条件不会撑长主渲染函数。
    fn render_header_actions(
        &self,
        current_interval: crate::monitor::SampleIntervalMode,
        drawer_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_1p5()
            .items_center()
            .flex_shrink_0()
            .child(self.render_sampling_controls(current_interval, cx))
            .child(self.render_view_actions(drawer_width, cx))
            .into_any_element()
    }

    /// 采样频率三档切换。
    fn render_sampling_controls(
        &self,
        current_interval: SampleIntervalMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                            cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(interval));
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
                            cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(interval));
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
                            cx.emit(crate::monitor::MonitorPanelEvent::RestartSampling(interval));
                        }
                    })),
            )
            .into_any_element()
    }

    /// 走势图形态、刷新、宽度、脱离与关闭动作。
    fn render_view_actions(&self, drawer_width: f32, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_1p5()
            .items_center()
            // 走势图形态切换（折线面积图 / 柱状图）
            .child(
                crate::widgets::IconButton::new(
                    "monitor-toggle-chart-type",
                    match self.state.chart_type {
                        SparklineChartType::Line => AppIcon::Ui(UiIcon::DATAFLOW),
                        SparklineChartType::Bar => AppIcon::Ui(UiIcon::DATAFLOW),
                    },
                    match self.state.chart_type {
                        SparklineChartType::Line => {
                            rust_i18n::t!("monitor.view.chart_line_tooltip").to_string()
                        }
                        SparklineChartType::Bar => {
                            rust_i18n::t!("monitor.view.chart_bar_tooltip").to_string()
                        }
                    },
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
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
                crate::widgets::IconButton::new(
                    "monitor-refresh",
                    AppIcon::Ui(UiIcon::REFRESH),
                    rust_i18n::t!("monitor.view.refresh_tooltip").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    let collapsed = this.state.collapse.processes;
                    cx.emit(crate::monitor::MonitorPanelEvent::SampleProcesses { collapsed });
                })),
            )
            .child(self.render_window_actions(drawer_width, cx))
            .into_any_element()
    }

    /// 宽度切换、脱离为标签页与关闭抽屉。
    fn render_window_actions(&self, drawer_width: f32, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_1p5()
            .items_center()
            // 宽度缩放切换芯片（快速切换 420px / 600px）
            .child(
                crate::widgets::IconButton::new(
                    "monitor-toggle-width",
                    if drawer_width > 480.0 {
                        AppIcon::Custom(CustomIcon::PANEL_NARROW)
                    } else {
                        AppIcon::Custom(CustomIcon::PANEL_WIDEN)
                    },
                    if drawer_width > 480.0 {
                        rust_i18n::t!("monitor.view.width_compact_tooltip").to_string()
                    } else {
                        rust_i18n::t!("monitor.view.width_wide_tooltip").to_string()
                    },
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
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
                crate::widgets::IconButton::new(
                    "monitor-maximize-to-tab",
                    AppIcon::Ui(UiIcon::UP),
                    rust_i18n::t!("monitor.view.detach_tooltip").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _window, cx| {
                    // 抽屉开合属于面板状态，先自己落定再回交外壳切标签。
                    this.set_open(false);
                    cx.notify();
                    cx.emit(crate::monitor::MonitorPanelEvent::MaximizeToTab);
                })),
            )
            // 关闭抽屉按钮
            .child(
                crate::widgets::IconButton::new(
                    "monitor-close",
                    AppIcon::Ui(UiIcon::CLOSE),
                    rust_i18n::t!("monitor.view.close_tooltip").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.state.open = false;
                    this.state.processes = std::rc::Rc::new(Vec::new());
                    this.state.process_revision = this.state.process_revision.wrapping_add(1);
                    this.state.process_order_cache = None;
                    cx.notify();
                })),
            )
            .into_any_element()
    }
}
