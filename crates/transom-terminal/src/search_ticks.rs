//! 检索刻度带的分桶计算：把命中行聚合到滚动条刻度上。

use crate::search::Hit;
use alacritty_terminal::index::Line;

/// 把命中映射到刻度带：按像素桶去重，避免上千个命中画出上千个矩形。
pub(crate) fn build_ticks(hits: &[Hit], top: Line, bottom: Line, tick_rows: usize) -> Vec<f32> {
    // 少于 2 个桶时无法给出可用的比例分母（窗格被压到 2px 以下的极端情况），直接不画。
    if tick_rows < 2 || bottom.0 <= top.0 {
        return Vec::new();
    }
    let span = (bottom.0 - top.0) as f32;
    let mut buckets = vec![false; tick_rows];
    for hit in hits {
        let fraction = (hit.start.line.0 - top.0) as f32 / span;
        let bucket = (fraction.clamp(0.0, 1.0) * (tick_rows - 1) as f32).round() as usize;
        buckets[bucket.min(tick_rows - 1)] = true;
    }
    buckets
        .iter()
        .enumerate()
        .filter(|(_, occupied)| **occupied)
        .map(|(bucket, _)| bucket as f32 / (tick_rows - 1) as f32)
        .collect()
}
