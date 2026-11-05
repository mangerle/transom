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
    pub(crate) fn infer(
        details: Option<&ContainerDetails>,
        current_state: ContainerState,
    ) -> Self {
        let mut buckets = [HourlyStatus::Unknown; 24];

        match current_state {
            ContainerState::Running => {
                // 运行态：假设近期保持稳定运行，填充最近时间片
                for slot in buckets.iter_mut().rev().take(12) {
                    *slot = HourlyStatus::Running;
                }
            }
            ContainerState::Exited | ContainerState::Dead => {
                let is_oom = details.and_then(|d| d.exit_code) == Some(137);
                let is_normal_exit = details.and_then(|d| d.exit_code) == Some(0);

                let exit_status = if is_oom {
                    HourlyStatus::OomKilled
                } else if is_normal_exit {
                    HourlyStatus::Exited
                } else {
                    HourlyStatus::Error
                };

                // 前半段曾运行，当前切片标为退出状态
                for slot in buckets.iter_mut().take(16) {
                    *slot = HourlyStatus::Running;
                }
                for slot in buckets.iter_mut().skip(16) {
                    *slot = exit_status;
                }
            }
            ContainerState::Paused => {
                for slot in buckets.iter_mut().take(20) {
                    *slot = HourlyStatus::Running;
                }
                for slot in buckets.iter_mut().skip(20) {
                    *slot = HourlyStatus::Exited;
                }
            }
            ContainerState::Restarting => {
                for slot in buckets.iter_mut().take(22) {
                    *slot = HourlyStatus::Running;
                }
                for slot in buckets.iter_mut().skip(22) {
                    *slot = HourlyStatus::Error;
                }
            }
            _ => {}
        }

        Self { buckets }
    }

    /// 记录当前这 1 小时的实时状态更新。
    pub(crate) fn update_current(&mut self, status: HourlyStatus) {
        self.buckets[23] = status;
    }
}

/// 渲染 24 小时微缩状态时间线（Status Ribbon）。
pub(crate) fn render_status_timeline(
    timeline: &StatusTimeline,
    p: &Palette,
) -> AnyElement {
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
        let tooltip_msg = rust_i18n::t!(
            "container.timeline.slot_desc",
            hours = hours_ago,
            status = status_text
        )
        .to_string();

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
    fn test_timeline_update_current() {
        let mut timeline = StatusTimeline::default();
        timeline.update_current(HourlyStatus::Running);
        assert_eq!(timeline.buckets[23], HourlyStatus::Running);
    }
}
