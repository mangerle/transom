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
            Self::Exited => p.muted_foreground.opacity(0.4),
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
    /// 创建全量状态一致的时间线。
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

        let created_ts = details
            .map(|d| d.created)
            .filter(|&t| t > 100_000_000);
        let started_ts = details
            .and_then(|d| parse_rfc3339_timestamp(d.started_at.as_deref()))
            .or(created_ts);
        let finished_ts = details
            .and_then(|d| parse_rfc3339_timestamp(d.finished_at.as_deref()));

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
            if let Some(created) = created_ts {
                if slot_end <= created {
                    *slot = HourlyStatus::Unknown;
                    continue;
                }
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
                    } else if let Some(started) = started_ts {
                        if slot_end >= started {
                            *slot = HourlyStatus::Running;
                        }
                    }
                }
                ContainerState::Restarting => {
                    if idx == 23 {
                        *slot = HourlyStatus::Error;
                    } else if let Some(started) = started_ts {
                        if slot_end >= started {
                            *slot = HourlyStatus::Running;
                        }
                    }
                }
                _ => {}
            }
        }

        Self { buckets }
    }

    /// 记录当前这 1 小时的实时状态更新。
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
        .bg(p.muted.opacity(0.12));

    for (idx, &status) in timeline.buckets.iter().enumerate() {
        let hours_ago = 24 - idx;
        let color = status.color(p);
        let status_text = status.label();
        let tooltip_msg = if hours_ago == 1 {
            rust_i18n::t!(
                "container.timeline.slot_desc_current",
                status = status_text
            )
            .to_string()
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
mod tests {
    use super::*;

    #[test]
    fn test_timeline_uniform() {
        let timeline = StatusTimeline::uniform(HourlyStatus::Running);
        for bucket in timeline.buckets {
            assert_eq!(bucket, HourlyStatus::Running);
        }
    }

    #[test]
    fn test_timeline_default_is_unknown() {
        let timeline = StatusTimeline::default();
        for bucket in timeline.buckets {
            assert_eq!(bucket, HourlyStatus::Unknown);
        }
    }

    #[test]
    fn test_timeline_infer_running() {
        let timeline = StatusTimeline::infer(None, ContainerState::Running);
        assert_eq!(timeline.buckets[23], HourlyStatus::Running);
    }

    #[test]
    fn test_timeline_infer_oom_killed() {
        let details = ContainerDetails {
            id: "cid-1".to_string(),
            name: "oom-app".to_string(),
            image: "test:latest".to_string(),
            image_id: "sha256:123".to_string(),
            command: "app".to_string(),
            entrypoint: Vec::new(),
            user: None,
            working_dir: None,
            env: Vec::new(),
            state: ContainerState::Exited,
            status: "Exited (137)".to_string(),
            created: 100,
            started_at: None,
            finished_at: None,
            exit_code: Some(137),
            health: None,
            health_details: None,
            restart_policy: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            compose_project: None,
            oom_killed: true,
            size_rw: 0,
            size_root_fs: 0,
            exposed_ports: Vec::new(),
            inspect: String::new(),
        };

        let timeline = StatusTimeline::infer(Some(&details), ContainerState::Exited);
        assert_eq!(timeline.buckets[23], HourlyStatus::OomKilled);
    }

    #[test]
    fn test_timeline_infer_newly_started_only_current_slot() {
        // 模拟当前时间 1730000000，容器是在 10 分钟前创建并启动的
        let now = 1730000000;
        let created = now - 600;
        let started_at = chrono::DateTime::from_timestamp(now - 600, 0)
            .unwrap()
            .to_rfc3339();

        let details = ContainerDetails {
            id: "cid-redis".to_string(),
            name: "redis".to_string(),
            image: "redis:latest".to_string(),
            image_id: "sha256:456".to_string(),
            command: "redis-server".to_string(),
            entrypoint: Vec::new(),
            user: None,
            working_dir: None,
            env: Vec::new(),
            state: ContainerState::Running,
            status: "Up 10 minutes".to_string(),
            created,
            started_at: Some(started_at),
            finished_at: None,
            exit_code: None,
            health: None,
            health_details: None,
            restart_policy: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            compose_project: None,
            oom_killed: false,
            size_rw: 0,
            size_root_fs: 0,
            exposed_ports: Vec::new(),
            inspect: String::new(),
        };

        let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Running, now);
        // 最近这 1 小时是运行中
        assert_eq!(timeline.buckets[23], HourlyStatus::Running);
        // 1 小时前至 24 小时前，容器尚未创建，必须为 Unknown（无记录），绝对不能显示为运行中
        for i in 0..23 {
            assert_eq!(
                timeline.buckets[i],
                HourlyStatus::Unknown,
                "第 {i} 小时槽应当为 Unknown，但被误判"
            );
        }
    }

    #[test]
    fn test_timeline_infer_long_running() {
        // 模拟运行了 3 天（72 小时）的容器
        let now = 1730000000;
        let created = now - 72 * 3600;
        let started_at = chrono::DateTime::from_timestamp(now - 72 * 3600, 0)
            .unwrap()
            .to_rfc3339();

        let details = ContainerDetails {
            id: "cid-long".to_string(),
            name: "postgres".to_string(),
            image: "postgres:16".to_string(),
            image_id: "sha256:789".to_string(),
            command: "postgres".to_string(),
            entrypoint: Vec::new(),
            user: None,
            working_dir: None,
            env: Vec::new(),
            state: ContainerState::Running,
            status: "Up 3 days".to_string(),
            created,
            started_at: Some(started_at),
            finished_at: None,
            exit_code: None,
            health: None,
            health_details: None,
            restart_policy: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            compose_project: None,
            oom_killed: false,
            size_rw: 0,
            size_root_fs: 0,
            exposed_ports: Vec::new(),
            inspect: String::new(),
        };

        let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Running, now);
        // 过去 24 小时全程处于稳定运行中，24 格全部为 Running
        for i in 0..24 {
            assert_eq!(timeline.buckets[i], HourlyStatus::Running);
        }
    }

    #[test]
    fn test_timeline_update_current() {
        let mut timeline = StatusTimeline::default();
        timeline.update_current(HourlyStatus::Running);
        assert_eq!(timeline.buckets[23], HourlyStatus::Running);
    }

    #[test]
    fn test_timeline_infer_recently_stopped_shows_exited_for_current_slot() {
        // 模拟当前时间，容器在 10 分钟前启动，在 2 分钟前退出
        let now = 1730000000;
        let created = now - 600;
        let started_at = chrono::DateTime::from_timestamp(now - 600, 0)
            .unwrap()
            .to_rfc3339();
        let finished_at = chrono::DateTime::from_timestamp(now - 120, 0)
            .unwrap()
            .to_rfc3339();

        let details = ContainerDetails {
            id: "cid-redis".to_string(),
            name: "redis".to_string(),
            image: "redis:latest".to_string(),
            image_id: "sha256:456".to_string(),
            command: "redis-server".to_string(),
            entrypoint: Vec::new(),
            user: None,
            working_dir: None,
            env: Vec::new(),
            state: ContainerState::Exited,
            status: "Exited (0) 2 minutes ago".to_string(),
            created,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            exit_code: Some(0),
            health: None,
            health_details: None,
            restart_policy: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            compose_project: None,
            oom_killed: false,
            size_rw: 0,
            size_root_fs: 0,
            exposed_ports: Vec::new(),
            inspect: String::new(),
        };

        let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Exited, now);
        // 当前时刻已停止，最右侧槽位（第 23 格）必须为 Exited，杜绝显示为 Running
        assert_eq!(timeline.buckets[23], HourlyStatus::Exited);
        // 1 小时前容器尚未创建，必须为 Unknown
        for i in 0..23 {
            assert_eq!(timeline.buckets[i], HourlyStatus::Unknown);
        }
    }

    #[test]
    fn test_timeline_infer_stopped_after_running() {
        // 模拟运行了 5 小时后在当前这 1 小时内退出的容器
        let now = 1730000000;
        let created = now - 5 * 3600;
        let started_at = chrono::DateTime::from_timestamp(now - 5 * 3600, 0)
            .unwrap()
            .to_rfc3339();
        let finished_at = chrono::DateTime::from_timestamp(now - 1800, 0)
            .unwrap()
            .to_rfc3339();

        let details = ContainerDetails {
            id: "cid-app".to_string(),
            name: "app".to_string(),
            image: "app:latest".to_string(),
            image_id: "sha256:789".to_string(),
            command: "app".to_string(),
            entrypoint: Vec::new(),
            user: None,
            working_dir: None,
            env: Vec::new(),
            state: ContainerState::Exited,
            status: "Exited (0) 30 minutes ago".to_string(),
            created,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            exit_code: Some(0),
            health: None,
            health_details: None,
            restart_policy: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            compose_project: None,
            oom_killed: false,
            size_rw: 0,
            size_root_fs: 0,
            exposed_ports: Vec::new(),
            inspect: String::new(),
        };

        let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Exited, now);
        // 当前时刻已退出
        assert_eq!(timeline.buckets[23], HourlyStatus::Exited);
        // 1~4 小时前的槽位全程在运行，必须为 Running
        for i in 19..23 {
            assert_eq!(
                timeline.buckets[i],
                HourlyStatus::Running,
                "第 {i} 小时槽应当为 Running"
            );
        }
    }
}
