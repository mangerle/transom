//! 偏好设置中的主机监控页：各板块默认展开/收起开关与采样偏好。

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::monitor::SampleIntervalMode;
use crate::palette::Palette;
use crate::settings::SettingsView;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

/// 监控板块默认展开开关的声明式目录。
struct MonitorSectionToggle {
    title_key: &'static str,
    desc_key: &'static str,
    element_id: &'static str,
    is_expanded: fn(&transom_core::MonitorSettings) -> bool,
    set_expanded: fn(&mut transom_core::MonitorSettings, bool),
}

const MONITOR_SECTION_TOGGLES: &[MonitorSectionToggle] = &[
    MonitorSectionToggle {
        title_key: "settings.monitor_cpu_expanded",
        desc_key: "settings.monitor_cpu_expanded_desc",
        element_id: "toggle-monitor-cpu-expanded",
        is_expanded: |m| m.cpu_expanded,
        set_expanded: |m, v| m.cpu_expanded = v,
    },
    MonitorSectionToggle {
        title_key: "settings.monitor_memory_expanded",
        desc_key: "settings.monitor_memory_expanded_desc",
        element_id: "toggle-monitor-memory-expanded",
        is_expanded: |m| m.memory_expanded,
        set_expanded: |m, v| m.memory_expanded = v,
    },
    MonitorSectionToggle {
        title_key: "settings.monitor_disk_expanded",
        desc_key: "settings.monitor_disk_expanded_desc",
        element_id: "toggle-monitor-disk-expanded",
        is_expanded: |m| m.disk_expanded,
        set_expanded: |m, v| m.disk_expanded = v,
    },
    MonitorSectionToggle {
        title_key: "settings.monitor_network_expanded",
        desc_key: "settings.monitor_network_expanded_desc",
        element_id: "toggle-monitor-network-expanded",
        is_expanded: |m| m.network_expanded,
        set_expanded: |m, v| m.network_expanded = v,
    },
    MonitorSectionToggle {
        title_key: "settings.monitor_ports_expanded",
        desc_key: "settings.monitor_ports_expanded_desc",
        element_id: "toggle-monitor-ports-expanded",
        is_expanded: |m| m.ports_expanded,
        set_expanded: |m, v| m.ports_expanded = v,
    },
    MonitorSectionToggle {
        title_key: "settings.monitor_services_expanded",
        desc_key: "settings.monitor_services_expanded_desc",
        element_id: "toggle-monitor-services-expanded",
        is_expanded: |m| m.services_expanded,
        set_expanded: |m, v| m.services_expanded = v,
    },
    MonitorSectionToggle {
        title_key: "settings.monitor_processes_expanded",
        desc_key: "settings.monitor_processes_expanded_desc",
        element_id: "toggle-monitor-processes-expanded",
        is_expanded: |m| m.processes_expanded,
        set_expanded: |m, v| m.processes_expanded = v,
    },
];

impl SettingsView {
    /// 监控板块默认展开/收起设置分组。
    pub(crate) fn render_settings_monitor_sections(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut content = v_flex().gap_2();
        for toggle in MONITOR_SECTION_TOGGLES {
            let checked = (toggle.is_expanded)(&app_settings(cx).monitor);
            let switch = Switch::new(toggle.element_id)
                .small()
                .checked(checked)
                .on_click(cx.listener(move |this, &checked, _, cx| {
                    update_app_settings(cx, |settings| {
                        (toggle.set_expanded)(&mut settings.monitor, checked);
                    });
                    let sections =
                        crate::monitor::MonitorSectionCollapse::from(&app_settings(cx).monitor);
                    cx.emit(crate::settings::SettingsEvent::MonitorSectionsChanged(
                        sections,
                    ));
                    let snapshot = app_settings(cx).clone();
                    this.persist_settings_reported(&snapshot, cx);
                    cx.notify();
                }))
                .into_any_element();
            content = content.child(crate::widgets::setting_row(
                toggle.title_key,
                Some(toggle.desc_key),
                switch,
                p,
            ));
        }
        content.into_any_element()
    }

    pub(crate) fn render_settings_monitor_behavior(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_width = app_settings(cx).monitor.drawer_width;
        let is_wide = current_width > 480.0;
        let current_interval = self.state.monitor_interval;

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
                        cx.emit(crate::settings::SettingsEvent::MonitorDrawerWidthChanged(
                            width,
                        ));
                        let settings_snapshot = app_settings(cx).clone();
                        this.persist_settings_reported(&settings_snapshot, cx);
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
                    |_this, mode, cx| {
                        cx.emit(crate::settings::SettingsEvent::MonitorIntervalChanged(mode));
                    },
                    cx,
                ),
                p,
            ))
            .into_any_element()
    }
}
