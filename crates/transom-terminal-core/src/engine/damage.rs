//! 终端网格损伤契约：渲染层据此只重建发生变化的视口行。
//!
//! 设计原理（Why）：损伤信息属于终端引擎的输出，不属于 GPUI 绘制层；
//! 放到核心 crate 后，核心测试可以验证脏行范围，而视图层只消费该结果。

/// 本帧需要重建绘制缓存的行集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DamageInfo {
    /// 全量重建：resize、滚动或外观变化导致行号与度量整体失效。
    Full,
    /// 仅重建给出的视口行号（已含 display_offset 换算）。
    Rows(Vec<usize>),
}
