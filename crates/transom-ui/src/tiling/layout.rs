//! 并排网格布局计算、评分算法与树构建。

use super::types::{MAX_TILING_SLOTS, TilingNode};
use gpui_kit::*;
use transom_core::{SplitAxis, SplitId};

const TARGET_CELL_ASPECT: f32 = 1.6;

/// 可用的手动网格：容量至少覆盖 `count`，且不超过槽位上限。
pub(crate) fn tiling_layout_options(count: usize) -> Vec<(usize, usize)> {
    let mut options = Vec::new();
    for rows in 1..=4 {
        for cols in 1..=4 {
            let capacity = rows * cols;
            if capacity >= count && capacity <= MAX_TILING_SLOTS {
                options.push((rows, cols));
            }
        }
    }
    options.sort_by_key(|(rows, cols)| (rows * cols, *rows, *cols));
    options
}

/// 自适应布局：在容量足够的网格里挑单元格长宽比最接近终端、空格最少的一个。
pub(crate) fn auto_tiling_grid(count: usize, window: &Window) -> (usize, usize) {
    best_grid(count, window_aspect(window))
}

/// 窗口宽高比；纯计算版本与 UI 共用，避免尺寸口径分叉。
pub(crate) fn window_aspect(window: &Window) -> f32 {
    let size = window.bounds().size;
    let width = size.width / px(1.0);
    let height = size.height / px(1.0);
    if height > 0.0 {
        (width / height).max(0.25)
    } else {
        1.0
    }
}

/// 网格评分：单元格越接近终端比例越好，空格位越少越好。
fn grid_score(rows: usize, cols: usize, count: usize, aspect: f32) -> f32 {
    let cell_aspect = aspect * rows as f32 / cols as f32;
    let empty = rows.saturating_mul(cols).saturating_sub(count) as f32;
    (cell_aspect - TARGET_CELL_ASPECT).abs() + empty * 0.8
}

/// 纯计算版本，便于单测覆盖不同窗口形状下的自适应结果。
pub(crate) fn best_grid(count: usize, aspect: f32) -> (usize, usize) {
    let mut best: Option<(f32, usize, usize)> = None;
    for rows in 1..=4 {
        for cols in 1..=4 {
            let capacity = rows * cols;
            if capacity < count || capacity > MAX_TILING_SLOTS {
                continue;
            }
            let score = grid_score(rows, cols, count, aspect);
            if best.is_none_or(|(best_score, _, _)| score < best_score) {
                best = Some((score, rows, cols));
            }
        }
    }
    best.map(|(_, rows, cols)| (rows, cols))
        .unwrap_or((1, count.clamp(1, MAX_TILING_SLOTS)))
}

/// 扩容时只在"行数不减、列数不减"的网格里选，保证原有格子的行列位置不变。
pub(crate) fn expand_grid(
    old_rows: usize,
    old_cols: usize,
    needed: usize,
    aspect: f32,
) -> Option<(usize, usize)> {
    let mut best: Option<(f32, usize, usize)> = None;
    for rows in old_rows..=4 {
        for cols in old_cols..=4 {
            let capacity = rows * cols;
            if capacity < needed || capacity > MAX_TILING_SLOTS {
                continue;
            }
            let score = grid_score(rows, cols, needed, aspect);
            if best.is_none_or(|(best_score, _, _)| score < best_score) {
                best = Some((score, rows, cols));
            }
        }
    }
    best.map(|(_, rows, cols)| (rows, cols))
}

/// 从行列数构建网格布局树。
pub(crate) fn build_tiling_grid(rows: usize, cols: usize) -> TilingNode {
    let cell = |row: usize, col: usize| TilingNode::Slot(row * cols + col);
    if cols >= rows {
        let columns = (0..cols)
            .map(|col| {
                if rows == 1 {
                    cell(0, col)
                } else {
                    stack(
                        SplitAxis::Vertical,
                        (0..rows).map(|row| cell(row, col)).collect(),
                    )
                }
            })
            .collect();
        stack(SplitAxis::Horizontal, columns)
    } else {
        let row_nodes = (0..rows)
            .map(|row| {
                if cols == 1 {
                    cell(row, 0)
                } else {
                    stack(
                        SplitAxis::Horizontal,
                        (0..cols).map(|col| cell(row, col)).collect(),
                    )
                }
            })
            .collect();
        stack(SplitAxis::Vertical, row_nodes)
    }
}

fn stack(axis: SplitAxis, mut children: Vec<TilingNode>) -> TilingNode {
    if children.len() == 1 {
        return children.remove(0);
    }
    TilingNode::Stack {
        id: SplitId::new(),
        axis,
        children,
    }
}
