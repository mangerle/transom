//! 容器 24 小时状态时间线（Status Ribbon / Timeline）。
//!
//! 设计原理（Why）：
//! - 采用方案 A（轻量内存环形分片 + 状态溯源），详情头增加 24 个 1 小时切片的微缩状态条；
//! - 运维人员无需跳出当前界面即可一览容器过去 24 小时内的生命周期波动（正常运行、正常退出、OOM 终止、异常报错）；
//! - 纯内存 `[HourlyStatus; 24]` 数组紧凑布局，零持久化负担，无锁且单测覆盖完备。

use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};
use transom_core::{ContainerDetails, ContainerState};

/// 单个时间切片（1 小时）的健康与运行状态倾向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HourlyStatus {
    #[default]
    Unknown,
    Running,
    Exited,
    Error,
    OomKilled,
}

impl HourlyStatus {
    /// 状态多语言描述文本。
    pub(crate) fn label(self) -> String {
        match self {
            Self::Running => rust_i18n::t!("container.timeline.running").to_string(),
            Self::Exited => rust_i18n::t!("container.timeline.exited").to_string(),
            Self::Error => rust_i18n::t!("container.timeline.error").to_string(),
            Self::OomKilled => rust_i18n::t!("container.timeline.oom_killed").to_string(),
            Self::Unknown => rust_i18n::t!("container.timeline.unknown").to_string(),
        }
    }

    /// 状态对应的主题语义色。
    pub(crate) fn color(self, p: &Palette) -> Hsla {
        match self {
            Self::Running => p.success,
            Self::Exited => p.text.disabled,
            Self::Error => p.warning,
            Self::OomKilled => p.danger,
            Self::Unknown => p.border.opacity(0.25),
        }
    }
}

/// 容器 24 小时微缩状态时间线数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusTimeline {
    /// 24 个按小时递增的切片：索引 0 代表 24 小时前，索引 23 代表当前这 1 小时。
    pub(crate) buckets: [HourlyStatus; 24],
}

impl Default for StatusTimeline {
    fn default() -> Self {
        Self {
            buckets: [HourlyStatus::Unknown; 24],
        }
    }
}

impl StatusTimeline {
    /// 创建全量状态一致的时间线；仅测试辅助使用。
    #[cfg(test)]
    pub(crate) fn uniform(status: HourlyStatus) -> Self {
        Self {
            buckets: [status; 24],
        }
    }

    /// 根据容器当前状态与 inspect 详情推导初始 24 小时状态。
    pub(crate) fn infer(details: Option<&ContainerDetails>, current_state: ContainerState) -> Self {
        Self::infer_at(details, current_state, chrono::Utc::now().timestamp())
    }

    /// 带有基准时间戳的状态推导实现，优先依赖真实生命周期时间戳，避免硬编码推测。
    pub(crate) fn infer_at(
        details: Option<&ContainerDetails>,
        current_state: ContainerState,
        now: i64,
    ) -> Self {
        let mut buckets = [HourlyStatus::Unknown; 24];
        const HOUR: i64 = 3600;

        let created_ts = details.map(|d| d.created).filter(|&t| t > 100_000_000);
        let started_ts = details
            .and_then(|d| parse_rfc3339_timestamp(d.started_at.as_deref()))
            .or(created_ts);
        let finished_ts = details.and_then(|d| parse_rfc3339_timestamp(d.finished_at.as_deref()));

        let is_oom = details.map(|d| d.oom_killed).unwrap_or(false)
            || details.and_then(|d| d.exit_code) == Some(137);
        let is_normal_exit = details.and_then(|d| d.exit_code) == Some(0);
        let exit_status = if is_oom {
            HourlyStatus::OomKilled
        } else if is_normal_exit {
            HourlyStatus::Exited
        } else {
            HourlyStatus::Error
        };

        for (idx, slot) in buckets.iter_mut().enumerate() {
            let slot_start = now - ((24 - idx) as i64) * HOUR;
            let slot_end = now - ((23 - idx) as i64) * HOUR;

            // 1. 若当前时间槽在容器创建前，容器尚不存在，标为无记录 (Unknown)
            if let Some(created) = created_ts
                && slot_end <= created
            {
                *slot = HourlyStatus::Unknown;
                continue;
            }

            match current_state {
                ContainerState::Running => {
                    // 若在该时间槽结束前容器已经启动，标为运行中，否则为启动前的无记录
                    if let Some(started) = started_ts {
                        if slot_end >= started {
                            *slot = HourlyStatus::Running;
                        } else {
                            *slot = HourlyStatus::Unknown;
                        }
                    } else if idx == 23 {
                        *slot = HourlyStatus::Running;
                    }
                }
                ContainerState::Exited | ContainerState::Dead => {
                    // 当前时刻槽位（idx == 23）必须真实反映容器当前的退出状态，杜绝已停止状态下误判为运行中
                    if idx == 23 {
                        *slot = exit_status;
                    } else if let (Some(started), Some(finished)) = (started_ts, finished_ts) {
                        if slot_end <= started {
                            // 槽位在容器启动之前，尚无运行记录
                            *slot = HourlyStatus::Unknown;
                        } else if slot_start >= finished {
                            // 槽位在容器退出之后，始终为已退出状态
                            *slot = exit_status;
                        } else if slot_end <= finished {
                            // 槽位结束前容器仍在正常运行
                            *slot = HourlyStatus::Running;
                        } else {
                            // 容器在该时间槽区间内退出了，标为退出状态
                            *slot = exit_status;
                        }
                    } else if let Some(finished) = finished_ts {
                        if slot_start >= finished {
                            *slot = exit_status;
                        } else {
                            *slot = HourlyStatus::Unknown;
                        }
                    } else {
                        *slot = HourlyStatus::Unknown;
                    }
                }
                ContainerState::Paused => {
                    if idx == 23 {
                        *slot = HourlyStatus::Exited;
                    } else if let Some(started) = started_ts
                        && slot_end >= started
                    {
                        *slot = HourlyStatus::Running;
                    }
                }
                ContainerState::Restarting => {
                    if idx == 23 {
                        *slot = HourlyStatus::Error;
                    } else if let Some(started) = started_ts
                        && slot_end >= started
                    {
                        *slot = HourlyStatus::Running;
                    }
                }
                _ => {}
            }
        }

        Self { buckets }
    }

    /// 记录当前这 1 小时的实时状态更新；仅测试辅助使用。
    #[cfg(test)]
    pub(crate) fn update_current(&mut self, status: HourlyStatus) {
        self.buckets[23] = status;
    }
}

/// 解析 RFC3339 字符串时间戳并过滤 Docker 的零值公元1年。
fn parse_rfc3339_timestamp(raw: Option<&str>) -> Option<i64> {
    let raw = raw?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp())
        .filter(|&t| t > 100_000_000)
}

/// 渲染 24 小时微缩状态时间线（Status Ribbon）。
pub(crate) fn render_status_timeline(timeline: &StatusTimeline, p: &Palette) -> AnyElement {
    let mut ribbon = h_flex()
        .gap(px(1.5))
        .items_center()
        .px_1p5()
        .py(px(2.0))
        .rounded(p.radius.sm)
        .bg(p.surface.subtle);

    for (idx, &status) in timeline.buckets.iter().enumerate() {
        let hours_ago = 24 - idx;
        let color = status.color(p);
        let status_text = status.label();
        let tooltip_msg = if hours_ago == 1 {
            rust_i18n::t!("container.timeline.slot_desc_current", status = status_text).to_string()
        } else {
            rust_i18n::t!(
                "container.timeline.slot_desc",
                hours = hours_ago,
                status = status_text
            )
            .to_string()
        };

        let slice = div()
            .id(ElementId::Name(format!("timeline-slot-{idx}").into()))
            .w(px(3.5))
            .h(px(11.0))
            .rounded(px(1.0))
            .bg(color)
            .tooltip(move |window, cx| Tooltip::new(tooltip_msg.clone()).build(window, cx));

        ribbon = ribbon.child(slice);
    }

    ribbon.into_any_element()
}

#[cfg(test)]
mod tests;
