//! 状态栏右侧指标簇实体：只持有指标快照，独立于外壳重绘。
//!
//! 设计原理（Why）：统计与流量样本以秒级频率到达，若每次都 `cx.notify()`
//! 外壳，整窗（标签栏、工作区、侧栏）都会重建。指标簇有自己的实体后，
//! 事件只更新快照并重绘这几个徽标；外壳仅在布局/可见性/活动标签变化时
//! 重绘。
//!
//! 代价与局限：拖拽排序会改全局设置并需要落盘，实体不持有外壳句柄，
//! 通过 [`StatusMetricsEvent::LayoutChanged`] 让外壳执行持久化。

use super::drag;
use super::items::{StatusData, render_status_item};
use crate::app::app_settings;
use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::*;
use transom_core::{NetworkTraffic, SystemStats};

/// 状态栏真正需要用到的统计字段：避免每帧克隆进程/端口等大列表。
#[derive(Clone, Default, PartialEq)]
pub(crate) struct StatusMetricsValues {
    pub(crate) cpu_percent: f32,
    pub(crate) mem_used_bytes: u64,
    pub(crate) mem_total_bytes: u64,
    pub(crate) disk_used_bytes: u64,
    pub(crate) disk_total_bytes: u64,
    pub(crate) os_name: String,
    pub(crate) uptime_secs: u64,
}

impl From<&SystemStats> for StatusMetricsValues {
    fn from(stats: &SystemStats) -> Self {
        Self {
            cpu_percent: stats.cpu_percent,
            mem_used_bytes: stats.mem_used_bytes,
            mem_total_bytes: stats.mem_total_bytes,
            disk_used_bytes: stats.disk_used_bytes,
            disk_total_bytes: stats.disk_total_bytes,
            os_name: stats.os_name.clone(),
            uptime_secs: stats.uptime_secs,
        }
    }
}

/// 当前活动标签的指标快照，由外壳在数据变化时注入。
#[derive(Clone, Default, PartialEq)]
pub(crate) struct StatusMetricsSnapshot {
    pub(crate) traffic: Option<NetworkTraffic>,
    pub(crate) stats: Option<StatusMetricsValues>,
    pub(crate) is_ssh: bool,
}

/// 指标簇对外事件：布局变化需要外壳落盘。
#[derive(Clone, Copy, Debug)]
pub(crate) enum StatusMetricsEvent {
    LayoutChanged,
}

pub(crate) struct StatusBarMetricsView {
    snapshot: StatusMetricsSnapshot,
    fallback: String,
    /// 状态文案为空时使用的版本兜底，只构造一次。
    version_fallback: String,
}

impl EventEmitter<StatusMetricsEvent> for StatusBarMetricsView {}

impl StatusBarMetricsView {
    pub(crate) fn new() -> Self {
        let version_fallback = format!("{} v{}", crate::APP_NAME, crate::APP_VERSION);
        Self {
            snapshot: StatusMetricsSnapshot::default(),
            fallback: version_fallback.clone(),
            version_fallback,
        }
    }

    /// 注入最新快照与无数据兜底文案；返回是否真的变化。
    pub(crate) fn set_snapshot(
        &mut self,
        snapshot: StatusMetricsSnapshot,
        status: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let expected = status.unwrap_or(&self.version_fallback);
        let changed = self.snapshot != snapshot || self.fallback != expected;
        if changed {
            self.snapshot = snapshot;
            self.fallback = expected.to_string();
            // 只有快照或状态文案真的变化才重绘指标簇；帧循环每帧都会注入
            // 同一快照，无条件 notify 会让六个徽标每帧重建一次。
            cx.notify();
        }
    }

    /// 当前快照是否已与给定数据一致；一致时调用方无需构造快照。
    pub(crate) fn matches(
        &self,
        traffic: Option<&NetworkTraffic>,
        stats: Option<&SystemStats>,
        is_ssh: bool,
        status: Option<&str>,
    ) -> bool {
        if self.snapshot.traffic.as_ref() != traffic || self.snapshot.is_ssh != is_ssh {
            return false;
        }
        let expected = status.unwrap_or(&self.version_fallback);
        if self.fallback != expected {
            return false;
        }
        match (self.snapshot.stats.as_ref(), stats) {
            (None, None) => true,
            (Some(values), Some(stats)) => {
                values.cpu_percent == stats.cpu_percent
                    && values.mem_used_bytes == stats.mem_used_bytes
                    && values.mem_total_bytes == stats.mem_total_bytes
                    && values.disk_used_bytes == stats.disk_used_bytes
                    && values.disk_total_bytes == stats.disk_total_bytes
                    && values.os_name == stats.os_name
                    && values.uptime_secs == stats.uptime_secs
            }
            _ => false,
        }
    }

    /// 布局已被拖拽或菜单修改：通知外壳持久化并重绘整条栏。
    pub(crate) fn layout_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(StatusMetricsEvent::LayoutChanged);
        cx.notify();
    }
}

impl Render for StatusBarMetricsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let items: Vec<transom_core::StatusItemId> =
            app_settings(cx).status_bar.visible_items().collect();
        let data = StatusData::from_snapshot(&self.snapshot);
        let view = cx.entity();
        let mut badges: Vec<AnyElement> = Vec::with_capacity(items.len());
        for id in items {
            if let Some(badge) = render_status_item(id, &data, &view, &p) {
                badges.push(badge);
            }
        }
        if badges.is_empty() {
            badges.push(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted_foreground)
                    .child(self.fallback.clone())
                    .into_any_element(),
            );
        }
        h_flex().gap_2().items_center().children(badges)
    }
}

/// 给指标徽标挂拖拽与菜单时使用的实体句柄别名。
pub(crate) type MetricsViewHandle = Entity<StatusBarMetricsView>;

/// 状态栏左侧空白落点也由指标实体的布局事件驱动落盘。
pub(crate) fn front_drop_zone(view: &MetricsViewHandle, p: &Palette) -> AnyElement {
    drag::metrics_front_drop_zone(view, p)
}
