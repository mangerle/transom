//! 滚动条刻度带的分桶计算：把一组网格行聚合到刻度带上。
//!
//! 检索命中与失败命令共用这套换算：两者的行号都以"相对视口底部"的 `Line` 表达，
//! 比例口径一致，才能画在同一条刻度带上而互相对不上。

use alacritty_terminal::index::Line;

/// 把行号映射到刻度带：按像素桶去重，避免上千个位置画出上千个矩形。
pub(crate) fn build_ticks(
    lines: impl Iterator<Item = i32>,
    top: Line,
    bottom: Line,
    tick_rows: usize,
) -> Vec<f32> {
    // 少于 2 个桶时无法给出可用的比例分母（窗格被压到 2px 以下的极端情况），直接不画。
    if tick_rows < 2 || bottom.0 <= top.0 {
        return Vec::new();
    }
    let span = (bottom.0 - top.0) as f32;
    let mut buckets = vec![false; tick_rows];
    for line in lines {
        let fraction = (line - top.0) as f32 / span;
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
