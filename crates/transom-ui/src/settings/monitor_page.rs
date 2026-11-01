//! 偏好设置中的主机监控页：各板块默认展开/收起开关与采样偏好。

use crate::app::AppShell;
use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::monitor::SampleIntervalMode;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

impl AppShell {
    /// 监控板块默认展开/收起设置分组。
    pub(crate) fn render_settings_monitor_sections(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.monitor_cpu_expanded",
                Some("settings.monitor_cpu_expanded_desc"),
                Switch::new("toggle-monitor-cpu-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.cpu_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.cpu_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.cpu = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_memory_expanded",
                Some("settings.monitor_memory_expanded_desc"),
                Switch::new("toggle-monitor-memory-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.memory_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.memory_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.memory = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_disk_expanded",
                Some("settings.monitor_disk_expanded_desc"),
                Switch::new("toggle-monitor-disk-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.disk_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.disk_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.disk = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_network_expanded",
                Some("settings.monitor_network_expanded_desc"),
                Switch::new("toggle-monitor-network-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.network_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.network_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.network = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_ports_expanded",
                Some("settings.monitor_ports_expanded_desc"),
                Switch::new("toggle-monitor-ports-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.ports_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.ports_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.ports = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_services_expanded",
                Some("settings.monitor_services_expanded_desc"),
                Switch::new("toggle-monitor-services-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.services_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.services_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.services = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_processes_expanded",
                Some("settings.monitor_processes_expanded_desc"),
                Switch::new("toggle-monitor-processes-expanded")
                    .small()
                    .checked(app_settings(cx).monitor.processes_expanded)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.monitor.processes_expanded = checked);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.state.collapse.processes = !checked);
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 监控抽屉尺寸与采样频率设置分组。
    pub(crate) fn render_settings_monitor_behavior(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_width = app_settings(cx).monitor.drawer_width;
        let is_wide = current_width > 480.0;
        let current_interval = self.state.monitor.read(cx).interval_mode();

        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.monitor_default_width",
                Some("settings.monitor_default_width_desc"),
                Self::setting_dropdown(
                    "dropdown-monitor-width",
                    if is_wide { 600.0f32 } else { 420.0f32 },
                    vec![
                        (
                            420.0f32,
                            rust_i18n::t!("monitor.settings.width_compact").to_string(),
                        ),
                        (
                            600.0f32,
                            rust_i18n::t!("monitor.settings.width_wide").to_string(),
                        ),
                    ],
                    |this, width, cx| {
                        update_app_settings(cx, |s| s.monitor.drawer_width = width);
                        this.state
                            .monitor
                            .update(cx, |view, _| view.set_drawer_width(width));
                        let settings_snapshot = app_settings(cx).clone();
                        this.state.persist_settings_reported(&settings_snapshot);
                        cx.notify();
                    },
                    cx,
                ),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.monitor_default_interval",
                Some("settings.monitor_default_interval_desc"),
                Self::setting_dropdown(
                    "dropdown-monitor-interval",
                    current_interval,
                    vec![
                        (
                            SampleIntervalMode::Fast,
                            rust_i18n::t!("monitor.settings.interval_fast").to_string(),
                        ),
                        (
                            SampleIntervalMode::Normal,
                            rust_i18n::t!("monitor.settings.interval_normal").to_string(),
                        ),
                        (
                            SampleIntervalMode::Eco,
                            rust_i18n::t!("monitor.settings.interval_eco").to_string(),
                        ),
                    ],
                    |this, mode, cx| {
                        this.set_monitor_interval_mode(mode, cx);
                    },
                    cx,
                ),
                p,
            ))
            .into_any_element()
    }
}
