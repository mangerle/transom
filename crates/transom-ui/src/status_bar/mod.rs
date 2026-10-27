//! 底部状态栏：左侧连接状态、右侧快捷命令入口与系统资源/网络指标。
//!
//! 状态栏按活动栏加侧栏的实测宽度切成两段；侧栏隐藏时不做切分。右侧徽标按
//! 用户布局动态排序，隐藏状态持久化；在场性由实时数据决定：没有网络采样、
//! 没有系统指标时对应条目直接不渲染，但顺序仍按全集保存，数据恢复后不会跳到
//! 别的位置。左侧连接状态固定，不参与排序。
//!
//! 所有数值徽标都给固定宽度的数值槽：网络速率会在 B/KB/MB/GB 与不同位数之间
//! 频繁变化，若宽度跟着内容走，整条状态栏会每秒抖动；固定槽让位置稳定，变化
//! 只发生在槽内。

mod drag;
mod format;

pub(crate) use format::{format_compact_size, format_rate};

use crate::app::{AppShell, TabStatus, TerminalTab};
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, NetworkTraffic, StatusItemId, SystemStats};

// 数值槽与徽标度量以 rem 表达（标准档 16px 基准），密度切换时随字号等比放大，
// 大字号下数字不会挤爆固定槽；底栏高度在 theme::tokens::size 集中定义。

const RATE_WIDTH: f32 = 2.5;
/// CPU 数值槽按最宽的 `100%` 预留，取值跨到三位数时不会把百分号截掉。
const CPU_VALUE_WIDTH: f32 = 2.25;
// 内存与磁盘槽按"1023G/1023G 100%"这一最宽组合预留：体积带上小数后比纯整数
// 多出两到三个字符，槽宽不足会走 truncate，把磁盘百分比整个截掉。
const MEMORY_VALUE_WIDTH: f32 = 6.5;
const DISK_VALUE_WIDTH: f32 = 6.5;
const OS_VALUE_WIDTH: f32 = 6.5;
const UPTIME_VALUE_WIDTH: f32 = 3.0;
pub(crate) const BADGE_HEIGHT: f32 = 1.25;

/// 当前活动标签能提供的指标快照。
struct StatusData<'a> {
    traffic: Option<&'a NetworkTraffic>,
    stats: Option<&'a SystemStats>,
    /// SSH 的网络速率来自连接传输层；其余会话（如本地终端）来自本机网卡。
    is_ssh: bool,
}

impl<'a> StatusData<'a> {
    fn from_tab(tab: Option<&'a TerminalTab>) -> Self {
        match tab {
            Some(tab) => Self {
                traffic: tab.network_traffic.as_ref(),
                stats: tab.system_stats.as_ref(),
                is_ssh: matches!(tab.session.connection, Connection::Ssh(_)),
            },
            None => Self {
                traffic: None,
                stats: None,
                is_ssh: false,
            },
        }
    }
}

impl AppShell {
    /// 渲染底栏。`divider_x` 为分界线横坐标，`0` 表示侧栏已隐藏、整条不分段。
    pub(crate) fn render_status_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
        divider_x: Pixels,
    ) -> AnyElement {
        let active_tab_info = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id));

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
            (color, format!("{}: {}", tab.display_name(), text))
        } else {
            (
                p.muted_foreground,
                rust_i18n::t!("status.ready").to_string(),
            )
        };

        let data = StatusData::from_tab(active_tab_info);
        let items: Vec<StatusItemId> = self.settings.status_bar.visible_items().collect();
        let app = cx.entity().clone();
        let mut right_elements = Vec::with_capacity(items.len());
        for id in items {
            if let Some(badge) = Self::render_status_item(id, &data, &app, p) {
                right_elements.push(badge);
            }
        }

        if right_elements.is_empty() {
            let text = self
                .status
                .clone()
                .unwrap_or_else(|| format!("{} v{}", crate::APP_NAME, crate::APP_VERSION));
            right_elements.push(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(text)
                    .into_any_element(),
            );
        }

        let pill_tooltip = status_text.clone();
        let connection_pill = h_flex()
            .id("status-connection")
            .min_w_0()
            .gap_2()
            .items_center()
            .h(rems(BADGE_HEIGHT))
            .px_2()
            .rounded(p.radius.md)
            .bg(p.muted)
            .border_1()
            .border_color(p.foreground.opacity(0.08))
            .cursor_default()
            .hover(|style| style.bg(p.foreground.opacity(0.06)))
            .tooltip(move |_window, cx| Tooltip::new(pill_tooltip.clone()).build(_window, cx))
            .child(
                div()
                    .flex_none()
                    .size(px(8.0))
                    .rounded_full()
                    .bg(status_dot_color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(p.foreground)
                    .child(status_text),
            );

        let snippets_entry = self.render_snippets_popover(p, cx);
        let broadcast_entry = self.render_broadcast_trigger(cx);
        let recording_badge = self.render_recording_badge(p, cx);
        let front_zone = drag::status_front_drop_zone(&app, p);
        let menu_app = app.clone();
        // ZMODEM 传输徽章优先于常规状态项：传输进行中用户最关心进度与取消。
        let zmodem_badge = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.render_zmodem_badge(tab_id, p, cx));
        let badges = h_flex()
            .flex_none()
            .gap_2()
            .items_center()
            .children(zmodem_badge)
            .children(right_elements);

        let mut bar = h_flex()
            .id("status-bar")
            .h(rems(size::STATUS_BAR_HEIGHT))
            .items_center()
            .bg(p.status_bar)
            .border_t_1()
            .border_color(p.border);

        if divider_x > px(0.0) {
            // 左段宽到侧栏右缘，连接状态过长时在段内截断；右段的左边框续上
            // 侧栏右边框，形成一条贯穿到窗口底边的分界线。
            bar = bar
                .child(
                    h_flex()
                        .w(divider_x)
                        .flex_none()
                        .h_full()
                        .items_center()
                        .pl_3()
                        .pr_2()
                        .child(h_flex().flex_1().min_w_0().child(connection_pill)),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .gap_2()
                        .pr_3()
                        .border_l_1()
                        .border_color(p.sidebar_border)
                        .child(snippets_entry)
                        .child(broadcast_entry)
                        .child(recording_badge)
                        .child(front_zone)
                        .child(badges),
                );
        } else {
            bar = bar
                .pl_3()
                .pr_3()
                .gap_2()
                .child(connection_pill)
                .child(snippets_entry)
                .child(broadcast_entry)
                .child(recording_badge)
                .child(front_zone)
                .child(badges);
        }

        // 空白区与左侧固定状态都落到整条状态栏的兜底菜单；徽标自己的菜单
        // 先命中并阻止冒泡，因此不会出现两层菜单同时打开。
        bar.context_menu(move |menu, _window, _cx| drag::status_bar_context_menu(menu, &menu_app))
            .into_any_element()
    }

    fn render_status_item(
        id: StatusItemId,
        data: &StatusData<'_>,
        app: &Entity<AppShell>,
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
                    .child(Self::rate_entry(
                        IconName::ArrowUp,
                        format_rate(traffic.upload_bytes_per_sec),
                        p,
                    ))
                    .child(Self::rate_entry(
                        IconName::ArrowDown,
                        format_rate(traffic.download_bytes_per_sec),
                        p,
                    ))
                    .into_any_element();
                Some(Self::status_badge(id, tooltip, content, app, p))
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
                let content = Self::icon_value(
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
                Some(Self::status_badge(id, tooltip, content, app, p))
            }
            StatusItemId::Memory => {
                let stats = data.stats.filter(|stats| stats.mem_total_bytes > 0)?;
                let used_gb = stats.mem_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let total_gb = stats.mem_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let mem_pct = (stats.mem_used_bytes as f64 / stats.mem_total_bytes as f64) * 100.0;
                let content = Self::icon_value(
                    IconName::MemoryStick,
                    format!(
                        "{}/{} {mem_pct:.0}%",
                        format_compact_size(stats.mem_used_bytes),
                        format_compact_size(stats.mem_total_bytes)
                    ),
                    MEMORY_VALUE_WIDTH,
                    p.foreground,
                    p.muted_foreground,
                );
                let tooltip = format!(
                    "{}: {used_gb:.1}G / {total_gb:.1}G ({mem_pct:.0}%)",
                    rust_i18n::t!("monitor.mem")
                );
                Some(Self::status_badge(id, tooltip, content, app, p))
            }
            StatusItemId::Disk => {
                let stats = data.stats.filter(|stats| stats.disk_total_bytes > 0)?;
                let used_gb = stats.disk_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let total_gb = stats.disk_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
                let disk_pct =
                    (stats.disk_used_bytes as f64 / stats.disk_total_bytes as f64) * 100.0;
                let content = Self::icon_value(
                    IconName::HardDrive,
                    format!(
                        "{}/{} {disk_pct:.0}%",
                        format_compact_size(stats.disk_used_bytes),
                        format_compact_size(stats.disk_total_bytes)
                    ),
                    DISK_VALUE_WIDTH,
                    p.foreground,
                    p.muted_foreground,
                );
                let tooltip = format!(
                    "{}: {used_gb:.1}G / {total_gb:.1}G ({disk_pct:.0}%)",
                    rust_i18n::t!("monitor.disk")
                );
                Some(Self::status_badge(id, tooltip, content, app, p))
            }
            StatusItemId::Os => {
                let stats = data.stats.filter(|stats| !stats.os_name.is_empty())?;
                let os_name = stats.os_name.clone();
                let content = Self::icon_value(
                    IconName::Monitor,
                    os_name.clone(),
                    OS_VALUE_WIDTH,
                    p.foreground,
                    p.muted_foreground,
                );
                Some(Self::status_badge(id, os_name, content, app, p))
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
                let content = Self::icon_value(
                    IconName::Clock,
                    up_text,
                    UPTIME_VALUE_WIDTH,
                    p.foreground,
                    p.muted_foreground,
                );
                Some(Self::status_badge(
                    id,
                    rust_i18n::t!("monitor.uptime").to_string(),
                    content,
                    app,
                    p,
                ))
            }
            StatusItemId::Unknown => None,
        }
    }

    /// 统一徽标外壳：底色、细边框、hover 与拖拽光标都在这里定义，条目只负责内容。
    fn status_badge(
        id: StatusItemId,
        tooltip: String,
        content: AnyElement,
        app: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let badge = div()
            .id(ElementId::Name(format!("status-{}", id.key()).into()))
            .relative()
            .h(rems(BADGE_HEIGHT))
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .rounded(p.radius.md)
            .bg(p.muted)
            .border_1()
            .border_color(p.foreground.opacity(0.10))
            .cursor_grab()
            .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx))
            .hover(|style| {
                style
                    .bg(p.foreground.opacity(0.10))
                    .border_color(p.foreground.opacity(0.28))
            })
            .active(|style| style.bg(p.foreground.opacity(0.16)))
            .child(content);
        drag::attach_status_drag(badge, id, app, p)
    }

    fn rate_entry(icon: IconName, value: String, p: &Palette) -> AnyElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(Icon::new(icon).xsmall().text_color(p.muted_foreground))
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
}

pub(crate) fn status_item_icon(id: StatusItemId) -> IconName {
    match id {
        StatusItemId::Network => IconName::ArrowUpDown,
        StatusItemId::Cpu => IconName::Cpu,
        StatusItemId::Memory => IconName::MemoryStick,
        StatusItemId::Disk => IconName::HardDrive,
        StatusItemId::Os => IconName::Monitor,
        StatusItemId::Uptime => IconName::Clock,
        StatusItemId::Unknown => IconName::CircleAlert,
    }
}

pub(crate) fn status_item_label(id: StatusItemId) -> String {
    let key = match id {
        StatusItemId::Network => "status.network",
        StatusItemId::Cpu => "monitor.cpu",
        StatusItemId::Memory => "monitor.mem",
        StatusItemId::Disk => "monitor.disk",
        StatusItemId::Os => "status.os",
        StatusItemId::Uptime => "monitor.uptime",
        StatusItemId::Unknown => "status.unknown",
    };
    rust_i18n::t!(key).to_string()
}

#[cfg(test)]
mod tests;
