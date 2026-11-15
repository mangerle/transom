//! 容器管理工作区的界面状态。
//!
//! 设计原理（Why）：工作区状态随标签存活，列表、选中项、详情页签、日志缓冲
//! 与 exec 终端都放在同一个结构里；事件按 `request_id` 找到所属标签后直接
//! 更新对应字段，界面不需要维护额外的订阅表。

use crate::monitor::{SampleIntervalMode, SparklineChartType};
use gpui_kit::Entity;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use transom_core::{
    ComposeProject, ContainerDetails, ContainerEndpoint, ContainerEngineInfo, ContainerEngineKind,
    ContainerEngineProfile, ContainerEngineRef, ContainerFileEntry, ContainerImageDetails,
    ContainerImageSummary, ContainerNetworkDetails, ContainerNetworkSummary, ContainerProcess,
    ContainerRequestId, ContainerStats, ContainerSummary, ContainerVolumeDetails,
    ContainerVolumeSummary, EngineProfileId, SessionColor, SessionIcon, TabId,
};
use transom_terminal::TerminalView;

mod build;
mod dialogs;
mod engine_form;
mod forward;
mod hub;
mod rows;
pub(crate) mod run_form;
mod section;
mod workspace;

pub(crate) use build::*;
pub(crate) use dialogs::*;
pub(crate) use engine_form::*;
pub(crate) use forward::*;
pub(crate) use hub::*;
pub(crate) use rows::*;
pub(crate) use run_form::*;
pub(crate) use section::*;
pub(crate) use workspace::*;

/// 日志缓冲上限：超出后丢弃最旧部分，避免长时间跟随把内存吃满。
pub(crate) const LOG_BUFFER_LIMIT: usize = 512 * 1024;
/// 单帧最多渲染的日志行数：只画尾部窗口，避免每次刷新重建上万个文本元素。
pub(crate) const LOG_RENDER_LIMIT: usize = 2000;

/// 容器统计固定保留的采样点数，与刷新间隔无关。
pub(crate) const STATS_HISTORY_LIMIT: usize = 120;
/// 默认只取尾部 500 行：一次性拉全量日志会把大容器的输出灌进界面缓冲。
pub(crate) const LOG_TAIL_DEFAULT: usize = 500;
/// 日志尾部行数档位：与 Portainer/lazydocker 的取数习惯一致。
pub(crate) const LOG_TAIL_CHOICES: [usize; 4] = [100, LOG_TAIL_DEFAULT, 5000, 0];

#[cfg(test)]
mod tests;
