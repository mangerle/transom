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
mod items;
pub(crate) mod metrics;

pub(crate) use format::{format_compact_size, format_rate};

pub(crate) use self::metrics::{StatusBarMetricsView, StatusMetricsSnapshot};
use crate::app::{AppShell, TabStatus};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{NetworkTraffic, StatusItemId};

// 数值槽与徽标度量以 rem 表达（标准档 16px 基准），密度切换时随字号等比放大，
// 大字号下数字不会挤爆固定槽；底栏高度在 theme::tokens::size 集中定义。

pub(crate) const BADGE_HEIGHT: f32 = 1.25;

impl AppShell {
    /// 是否有标签正在录制：录制中必须保住停止入口。
    pub(crate) fn is_any_tab_recording(&self, cx: &App) -> bool {
        self.state
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| tab.terminal())
            .any(|term| term.read(cx).is_recording())
    }

    /// 渲染底栏。`divider_x` 为分界线横坐标，`0` 表示侧栏已隐藏、整条不分段。
    pub(crate) fn render_status_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
        divider_x: Pixels,
    ) -> AnyElement {
        let connection_pill = self.render_connection_pill(p);
        let app = cx.entity().clone();
        let metrics = self.state.status_metrics.clone();

        // 三个会话作用域入口（快捷命令、广播、录制）只在存在终端标签时出现：
        // 没有终端时它们点了要么静默返回、要么打开一个没有目标的输入条、要么
        // 到执行步才报错，留着只会误导。判定取"任意标签有终端"而不是"活动标签
        // 有终端"：广播的目标本来就跨标签多选，且图标不该随切换标签跳动。
        // 三条兜底：已打开的卡片与输入条、以及正在录制的标签，都不允许因为
        // 图标消失而失去关闭/停止的入口。
        let session_scoped = self.state.workspace.has_any_terminal();
        let snippets_entry = (session_scoped || self.state.modals.tools.snippet_panel.open)
            .then(|| self.render_snippets_popover(p, cx));
        let broadcast_entry = (session_scoped || self.state.broadcast.open)
            .then(|| self.render_broadcast_trigger(cx));
        let recording_badge = (session_scoped || self.is_any_tab_recording(cx))
            .then(|| self.render_recording_badge(p, cx));
        let front_zone = metrics::front_drop_zone(&self.state.status_metrics, p);
        let menu_app = app.clone();
        // ZMODEM 传输徽章优先于常规状态项：传输进行中用户最关心进度与取消。
        let zmodem_badge = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.render_zmodem_badge(tab_id, p, cx));
        let badges = h_flex()
            .flex_none()
            .gap_2()
            .items_center()
            .children(zmodem_badge)
            .child(metrics);

        let mut bar = h_flex()
            .id("status-bar")
            .w_full()
            .h(rems(size::STATUS_BAR_HEIGHT))
            .bg(p.status_bar)
            .border_color(p.divider(p.border))
            .text_xs()
            .text_color(p.muted_foreground)
            .items_center()
            .context_menu(move |menu, _window, _cx| drag::status_bar_context_menu(menu, &menu_app));
        bar = bar.divider_edge(DividerEdge::Top, p.divider_width);

        if divider_x > px(0.0) {
            // 左段宽到侧栏右缘，连接状态过长时在段内截断；右段的左边框续上
            // 侧栏右边框，形成一条贯穿到窗口底边的分界线。
            let mut right_segment = h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_center()
                .gap_2()
                .pr_3()
                .border_color(p.divider(p.border))
                .children(snippets_entry)
                .children(broadcast_entry)
                .children(recording_badge)
                .child(front_zone)
                .child(badges);
            right_segment = right_segment.divider_edge(DividerEdge::Left, p.divider_width);
            bar = bar
                .child(
                    h_flex()
                        .w(divider_x)
                        .flex_none()
                        .h_full()
                        .items_center()
                        .pl_2()
                        .pr_2()
                        .child(h_flex().flex_1().min_w_0().child(connection_pill)),
                )
                .child(right_segment);
        } else {
            bar = bar.child(
                h_flex()
                    .flex_1()
                    .h_full()
                    .items_center()
                    .pl_2()
                    .pr_3()
                    .gap_2()
                    .child(connection_pill)
                    .children(snippets_entry)
                    .children(broadcast_entry)
                    .children(recording_badge)
                    .child(front_zone)
                    .child(badges),
            );
        }

        bar.into_any_element()
    }
}

impl AppShell {
    /// 把活动标签的指标快照推进状态栏指标实体；无可见数据时给出版本兜底。
    pub(crate) fn refresh_status_metrics(&mut self, cx: &mut Context<Self>) {
        let tab = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id));
        let snapshot = crate::status_bar::StatusMetricsSnapshot {
            traffic: tab.and_then(|tab| tab.network_traffic()).cloned(),
            stats: tab
                .and_then(|tab| tab.system_stats())
                .map(metrics::StatusMetricsValues::from),
            is_ssh: tab.is_some_and(|tab| {
                matches!(tab.session.connection, transom_core::Connection::Ssh(_))
            }),
        };
        let status = self.state.status.as_deref();
        self.state.status_metrics.update(cx, |view, cx| {
            view.set_snapshot(snapshot, status, cx);
        });
    }
}

impl AppShell {
    /// 左侧连接状态胶囊：活动标签名、连接状态点与状态文案。
    fn render_connection_pill(&self, p: &Palette) -> AnyElement {
        let active_tab_info = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id));

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

        let pill_tooltip = status_text.clone();
        let connection_pill = h_flex()
            .id("status-connection")
            .min_w_0()
            .gap_2()
            .items_center()
            .h(rems(BADGE_HEIGHT))
            .px_2()
            .rounded(p.radius.md)
            .bg(p.secondary)
            .border_1()
            .border_color(p.border)
            .cursor_default()
            .hover(|style| style.bg(p.secondary_hover))
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
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.secondary_foreground)
                    .child(status_text),
            );
        connection_pill.into_any_element()
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
