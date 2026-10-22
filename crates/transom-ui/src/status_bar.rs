//! 底部状态栏（系统资源监控指标展示）与顶部错误横幅渲染模块。

use crate::app::{AppShell, TabStatus};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

impl AppShell {
    pub(crate) fn render_status_bar(&self, p: &Palette) -> AnyElement {
        let active_tab_info = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id));

        let (status_dot_color, status_text) = if let Some(tab) = active_tab_info {
            let (color, text) = match &tab.status {
                TabStatus::Connecting => (
                    p.muted_foreground,
                    rust_i18n::t!("status.connecting").to_string(),
                ),
                TabStatus::Connected => (p.success, rust_i18n::t!("status.connected").to_string()),
                TabStatus::Disconnected => {
                    (p.danger, rust_i18n::t!("status.disconnected").to_string())
                }
                TabStatus::Failed(_) => (p.danger, rust_i18n::t!("status.failed").to_string()),
            };
            (color, format!("{}: {}", tab.title, text))
        } else {
            (
                p.muted_foreground,
                rust_i18n::t!("status.ready").to_string(),
            )
        };

        let mut right_elements = Vec::new();

        if let Some(tab) = active_tab_info
            && let Some(stats) = &tab.system_stats
        {
            let cpu_color = if stats.cpu_percent > 85.0 {
                p.danger
            } else if stats.cpu_percent > 65.0 {
                p.sidebar_accent_foreground
            } else {
                p.foreground
            };
            let cpu_badge = h_flex()
                .gap_1p5()
                .items_center()
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(p.muted)
                .child(div().size(px(6.0)).rounded_full().bg(cpu_color))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cpu_color)
                        .child(format!(
                            "{}: {:.1}%",
                            rust_i18n::t!("monitor.cpu"),
                            stats.cpu_percent
                        )),
                )
                .into_any_element();
            right_elements.push(cpu_badge);

            if stats.mem_total_bytes > 0 {
                let used_gb = stats.mem_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let total_gb = stats.mem_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let mem_pct = (stats.mem_used_bytes as f64 / stats.mem_total_bytes as f64) * 100.0;
                let mem_badge = h_flex()
                    .gap_1p5()
                    .items_center()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(p.muted)
                    .child(div().text_xs().text_color(p.foreground).child(format!(
                        "{}: {:.1}G/{:.1}G ({:.0}%)",
                        rust_i18n::t!("monitor.mem"),
                        used_gb,
                        total_gb,
                        mem_pct
                    )))
                    .into_any_element();
                right_elements.push(mem_badge);
            }

            if stats.disk_total_bytes > 0 {
                let used_gb = stats.disk_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let total_gb = stats.disk_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let disk_pct =
                    (stats.disk_used_bytes as f64 / stats.disk_total_bytes as f64) * 100.0;
                let disk_badge = h_flex()
                    .gap_1p5()
                    .items_center()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(p.muted)
                    .child(div().text_xs().text_color(p.foreground).child(format!(
                        "{}: {:.1}G/{:.1}G ({:.0}%)",
                        rust_i18n::t!("monitor.disk"),
                        used_gb,
                        total_gb,
                        disk_pct
                    )))
                    .into_any_element();
                right_elements.push(disk_badge);
            }

            if stats.uptime_secs > 0 || !stats.os_name.is_empty() {
                let days = stats.uptime_secs / 86400;
                let hours = (stats.uptime_secs % 86400) / 3600;
                let mins = (stats.uptime_secs % 3600) / 60;
                let up_text = if days > 0 {
                    format!("{}d {}h", days, hours)
                } else if hours > 0 {
                    format!("{}h {}m", hours, mins)
                } else if mins > 0 {
                    format!("{}m", mins)
                } else {
                    "< 1m".to_string()
                };
                let os_badge = h_flex()
                    .gap_1p5()
                    .items_center()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(p.muted)
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("{} | Up {}", stats.os_name, up_text)),
                    )
                    .into_any_element();
                right_elements.push(os_badge);
            }
        }

        if right_elements.is_empty() {
            let text = self
                .status
                .clone()
                .unwrap_or_else(|| "Transom v0.1.0".to_string());
            right_elements.push(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(text)
                    .into_any_element(),
            );
        }

        h_flex()
            .h(px(28.0))
            .px_3()
            .items_center()
            .justify_between()
            .bg(p.status_bar)
            .border_t_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().size(px(8.0)).rounded_full().bg(status_dot_color))
                    .child(div().text_xs().text_color(p.foreground).child(status_text)),
            )
            .child(h_flex().gap_2().items_center().children(right_elements))
            .into_any_element()
    }

    pub(crate) fn render_error_banner(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .bg(p.danger)
            .text_color(p.danger_foreground)
            .child(
                div()
                    .text_sm()
                    .child(self.error.clone().unwrap_or_default()),
            )
            .child(
                Button::new("dismiss-error")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}
