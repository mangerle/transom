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
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{NetworkTraffic, StatusItemId, TabId};

// 数值槽与徽标度量以 rem 表达（标准档 16px 基准），密度切换时随字号等比放大，
// 大字号下数字不会挤爆固定槽；底栏高度在 theme::tokens::size 集中定义。

pub(crate) const BADGE_HEIGHT: f32 = 1.25;

/// 状态栏左侧入口元素：可选入口与 ZMODEM 徽章。
struct StatusBarEntries {
    snippets: Option<AnyElement>,
    broadcast: Option<AnyElement>,
    recording: Option<AnyElement>,
    zmodem: Option<AnyElement>,
}

impl AppShell {
    /// 渲染底栏。`divider_x` 为分界线横坐标，`0` 表示侧栏已隐藏、整条不分段。
    /// 状态栏左侧会话作用域入口：片段、广播、录制与 ZMODEM 徽章。
    fn status_bar_entries(&mut self, cx: &mut Context<Self>, p: &Palette) -> StatusBarEntries {
        // 三个入口都直接作用于当前终端。容器管理、录像回放与主机监控标签没有
        // 可输入终端，保留入口只会产生无响应点击，所以按活动标签能力显隐。
        let terminal_scoped = self.state.active_tab_has_terminal();
        let snippets_entry = terminal_scoped.then(|| self.render_snippets_popover(p, cx));
        let broadcast_entry = terminal_scoped.then(|| self.render_broadcast_trigger(cx));
        let recording_badge = terminal_scoped.then(|| self.render_recording_badge(p, cx));
        let zmodem = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.render_zmodem_badge(tab_id, p, cx));
        StatusBarEntries {
            snippets: snippets_entry,
            broadcast: broadcast_entry,
            recording: recording_badge,
            zmodem,
        }
    }

    pub(crate) fn render_status_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
        divider_x: Pixels,
    ) -> AnyElement {
        let connection_pill = self.render_connection_pill(p);
        let app = cx.entity().clone();
        let metrics = self.state.status_metrics.clone();

        let entries = self.status_bar_entries(cx, p);
        let front_zone = metrics::front_drop_zone(&self.state.status_metrics, p);
        let menu_app = app.clone();
        let badges = h_flex()
            .flex_none()
            .gap_2()
            .items_center()
            .children(entries.zmodem)
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

        let active_git_tab = self
            .state
            .workspace
            .active_tab
            .filter(|tab_id| self.git_state_of_tab(*tab_id).is_some())
            .or_else(|| {
                self.state
                    .workspace
                    .tabs
                    .iter()
                    .find_map(|t| if t.is_git() { Some(t.id) } else { None })
            });

        let make_git_log_btn = |id: TabId, app_entity: Entity<AppShell>| {
            Button::new("status-bar-git-log")
                .ghost()
                .xsmall()
                .icon(IconName::GitCommitHorizontal)
                .tooltip(rust_i18n::t!("git.view.log").to_string())
                .on_click(move |_, _, cx| {
                    app_entity.update(cx, |this, cx| {
                        this.open_git_log_modal(id, cx);
                    });
                })
        };

        if divider_x > px(0.0) {
            // 左段宽到侧栏右缘，连接状态过长时在段内截断；右段的左边框续上
            // 侧栏右边框，形成一条贯穿到窗口底边的分界线。
            let mut right_segment = h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_center()
                .gap_2()
                .pl_1()
                .pr_3()
                .border_color(p.divider(p.border))
                .children(active_git_tab.map(|id| make_git_log_btn(id, app.clone())))
                .children(entries.snippets)
                .children(entries.broadcast)
                .children(entries.recording)
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
                    .children(active_git_tab.map(|id| make_git_log_btn(id, app.clone())))
                    .children(entries.snippets)
                    .children(entries.broadcast)
                    .children(entries.recording)
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
        let traffic = tab.and_then(|tab| tab.network_traffic());
        let stats = tab.and_then(|tab| tab.system_stats());
        let is_ssh = tab
            .is_some_and(|tab| matches!(tab.session.connection, transom_core::Connection::Ssh(_)));
        let status = self.state.status.as_deref();
        // 先借用比较：未变化时不构造快照，避免每帧克隆系统名与流量样本。
        if self
            .state
            .status_metrics
            .read(cx)
            .matches(traffic, stats, is_ssh, status)
        {
            return;
        }
        let snapshot = crate::status_bar::StatusMetricsSnapshot {
            traffic: traffic.cloned(),
            stats: stats.map(metrics::StatusMetricsValues::from),
            is_ssh,
        };
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
