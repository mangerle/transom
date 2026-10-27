//! 活动栏度量常量与尺寸计算。

use crate::theme::tokens::size;

/// 活动栏的布局度量统一从这里取值。
///
/// 设计原理（Why）：
/// - 实现初衷：按钮尺寸与间距同时决定条宽、命中区和插入线位置，单独暴露像素输入框
///   会制造一组需要互相校验的状态。宽度、按钮与图标以 rem 表达，随密度档位整体缩放；
///   间距为像素语义微度量，保留 px 不随密度变化；
/// - 核心优势：集中定义布局参数，避免散落在各处绘制方法中产生不一致；
/// - 代价与局限：图标尺寸与会话树行图标（`sidebar::tree` 的 `ROW_ICON_SIZE`）需手动保持同步。
#[derive(Clone, Copy)]
pub(super) struct ActivityBarMetrics {
    pub(super) width: f32,
    pub(super) button: f32,
    pub(super) icon: f32,
    pub(super) gap: f32,
    pub(super) padding_y: f32,
}

pub(super) const METRICS: ActivityBarMetrics = ActivityBarMetrics {
    width: size::ACTIVITY_BAR_WIDTH,
    button: size::ACTIVITY_BUTTON_SIZE,
    icon: size::ACTIVITY_ICON_SIZE,
    gap: 4.0,
    padding_y: 4.0,
};
