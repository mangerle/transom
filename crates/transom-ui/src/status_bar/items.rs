//! 状态栏右侧指标条目与徽标外壳。

use super::metrics::{StatusBarMetricsView, StatusMetricsSnapshot};
use super::*;
use crate::status_bar::BADGE_HEIGHT;
use gpui_kit::base::h_flex;

const RATE_WIDTH: f32 = 2.5;
/// CPU 数值槽按最宽的 `100%` 预留，取值跨到三位数时不会把百分号截掉。
const CPU_VALUE_WIDTH: f32 = 2.25;
// 内存与磁盘槽按"1023G/1023G 100%"这一最宽组合预留：体积带上小数后比纯整数
// 多出两到三个字符，槽宽不足会走 truncate，把磁盘百分比整个截掉。
const MEMORY_VALUE_WIDTH: f32 = 6.5;
const DISK_VALUE_WIDTH: f32 = 6.5;
const OS_VALUE_WIDTH: f32 = 6.5;
const UPTIME_VALUE_WIDTH: f32 = 3.0;
/// 当前活动标签能提供的指标快照。
pub(super) struct StatusData<'a> {
    traffic: Option<&'a NetworkTraffic>,
    stats: Option<&'a super::metrics::StatusMetricsValues>,
    /// SSH 的网络速率来自连接传输层；其余会话（如本地终端）来自本机网卡。
    is_ssh: bool,
}

impl<'a> StatusData<'a> {
    pub(super) fn from_snapshot(snapshot: &'a StatusMetricsSnapshot) -> Self {
        Self {
            traffic: snapshot.traffic.as_ref(),
            stats: snapshot.stats.as_ref(),
            is_ssh: snapshot.is_ssh,
        }
    }
}

pub(super) fn render_status_item(
    id: StatusItemId,
    data: &StatusData<'_>,
    view: &Entity<StatusBarMetricsView>,
    p: &Palette,
) -> Option<AnyElement> {
    match id {
        StatusItemId::Network => {
            let traffic = data.traffic?;
            let tooltip = if data.is_ssh {
                rust_i18n::t!("status.network_tooltip_ssh").to_string()
            } else {
                rust_i18n::t!("status.network_tooltip_local").to_string()
            };
            let content = h_flex()
                .gap_2()
                .items_center()
                .child(rate_entry(
                    IconName::ArrowUp,
                    format_rate(traffic.upload_bytes_per_sec),
                    p,
                ))
                .child(rate_entry(
                    IconName::ArrowDown,
                    format_rate(traffic.download_bytes_per_sec),
                    p,
                ))
                .into_any_element();
            Some(status_badge(id, tooltip, content, view, p))
        }
        StatusItemId::Cpu => {
            let stats = data.stats?;
            let cpu_color = if stats.cpu_percent > 85.0 {
                p.danger
            } else if stats.cpu_percent > 65.0 {
                p.sidebar_accent_foreground
            } else {
                p.foreground
            };
            let content = icon_value(
                IconName::Cpu,
                format!("{:.0}%", stats.cpu_percent),
                CPU_VALUE_WIDTH,
                cpu_color,
                cpu_color,
            );
            // 槽位只为稳定宽度服务；tooltip 始终给出精确百分比，避免极端值被裁掉后读不到。
            let tooltip = format!(
                "{}: {:.0}%",
                rust_i18n::t!("monitor.cpu"),
                stats.cpu_percent
            );
            Some(status_badge(id, tooltip, content, view, p))
        }
        StatusItemId::Memory => {
            let stats = data.stats.filter(|stats| stats.mem_total_bytes > 0)?;
            let used_gb = stats.mem_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
            let total_gb = stats.mem_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
            let mem_pct = (stats.mem_used_bytes as f64 / stats.mem_total_bytes as f64) * 100.0;
            let content = icon_value(
                IconName::MemoryStick,
                format!(
                    "{}/{} {mem_pct:.0}%",
                    format_compact_size(stats.mem_used_bytes),
                    format_compact_size(stats.mem_total_bytes)
                ),
                MEMORY_VALUE_WIDTH,
                p.foreground,
                p.foreground,
            );
            let tooltip = format!(
                "{}: {used_gb:.1}G / {total_gb:.1}G ({mem_pct:.0}%)",
                rust_i18n::t!("monitor.mem")
            );
            Some(status_badge(id, tooltip, content, view, p))
        }
        StatusItemId::Disk => {
            let stats = data.stats.filter(|stats| stats.disk_total_bytes > 0)?;
            let used_gb = stats.disk_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
            let total_gb = stats.disk_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
            let disk_pct = (stats.disk_used_bytes as f64 / stats.disk_total_bytes as f64) * 100.0;
            let content = icon_value(
                IconName::HardDrive,
                format!(
                    "{}/{} {disk_pct:.0}%",
                    format_compact_size(stats.disk_used_bytes),
                    format_compact_size(stats.disk_total_bytes)
                ),
                DISK_VALUE_WIDTH,
                p.foreground,
                p.foreground,
            );
            let tooltip = format!(
                "{}: {used_gb:.1}G / {total_gb:.1}G ({disk_pct:.0}%)",
                rust_i18n::t!("monitor.disk")
            );
            Some(status_badge(id, tooltip, content, view, p))
        }
        StatusItemId::Os => {
            let stats = data.stats.filter(|stats| !stats.os_name.is_empty())?;
            let os_name = stats.os_name.clone();
            let content = icon_value(
                IconName::Monitor,
                os_name.clone(),
                OS_VALUE_WIDTH,
                p.foreground,
                p.foreground,
            );
            Some(status_badge(id, os_name, content, view, p))
        }
        StatusItemId::Uptime => {
            let stats = data
                .stats
                .filter(|stats| stats.uptime_secs > 0 || !stats.os_name.is_empty())?;
            let days = stats.uptime_secs / 86400;
            let hours = (stats.uptime_secs % 86400) / 3600;
            let mins = (stats.uptime_secs % 3600) / 60;
            let up_text = if days > 0 {
                format!("{days}d{hours}h")
            } else if hours > 0 {
                format!("{hours}h{mins}m")
            } else if mins > 0 {
                format!("{mins}m")
            } else {
                "<1m".to_string()
            };
            let content = icon_value(
                IconName::Clock,
                up_text,
                UPTIME_VALUE_WIDTH,
                p.foreground,
                p.foreground,
            );
            Some(status_badge(
                id,
                rust_i18n::t!("monitor.uptime").to_string(),
                content,
                view,
                p,
            ))
        }
        StatusItemId::Unknown => None,
    }
}

/// 统一徽标外壳：使用 Tag 组件规范底色与边框，条目负责内容。
fn status_badge(
    id: StatusItemId,
    tooltip: String,
    content: AnyElement,
    view: &Entity<StatusBarMetricsView>,
    p: &Palette,
) -> AnyElement {
    let tag = Tag::secondary()
        .h(rems(BADGE_HEIGHT))
        .gap_1()
        .px_1p5()
        .rounded(p.radius.md)
        .child(content);
    let badge = div()
        .id(ElementId::Name(format!("status-{}", id.key()).into()))
        .relative()
        .cursor_grab()
        .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx))
        .child(tag);
    drag::attach_status_drag_metrics(badge, id, view, p)
}

fn rate_entry(icon: IconName, value: String, p: &Palette) -> AnyElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(Icon::new(icon).xsmall().text_color(p.foreground))
        .child(
            div().w(rems(RATE_WIDTH)).flex().child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.foreground)
                    .child(value),
            ),
        )
        .into_any_element()
}

/// 固定宽度的数值槽：数值紧贴图标，槽位剩余空间留到右侧。
///
/// 不用右对齐：内存、运行时间这类短数值会在图标和数字之间空出一大块，
/// 看起来像排版断裂。槽宽固定保证徽标整体不抖，左对齐保证数字始终贴着图标。
fn icon_value(
    icon: IconName,
    value: String,
    width: f32,
    value_color: Hsla,
    icon_color: Hsla,
) -> AnyElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(Icon::new(icon).xsmall().text_color(icon_color))
        .child(
            div().w(rems(width)).flex().child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(value_color)
                    .child(value),
            ),
        )
        .into_any_element()
}
