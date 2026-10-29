//! 行级文件差异契约：Myers 行差异、变更块（hunk）与单向合并计算。
//!
//! 设计原理（Why）：
//! - 差异算法选用 `similar` 的 Myers 实现：它是组件库 diff 视图的同源依赖，
//!   不引入第二套算法口径；本模块只把结果收敛成 UI 能直接消费的 hunk 区间，
//!   渲染与虚拟化交给视图层，领域层不持有任何界面类型；
//! - 只保存 hunk 的行区间而不复制每一行文本：合并时按区间从原文切片，
//!   大文件不必为差异结果再存一份内容；代价是调用方必须把生成差异时的
//!   左右原文一并传回；
//! - 合并只允许"单向"：一次把一侧的某个变更块应用到另一侧，不做双向自动
//!   合并，也不猜测冲突。用户明确点哪一块就只改哪一块。

use similar::{DiffOp, TextDiff};

/// 默认上下文行数：与常见 diff 工具一致，足够看懂改动位置。
pub const DEFAULT_CONTEXT_LINES: usize = 3;

/// 一个连续变更块在左右两侧的行区间（0 基，`len` 可为 0 表示纯插入/纯删除）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

/// 单向合并方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeDirection {
    /// 把左侧变更块应用到右侧（本地 → 远端）。
    LeftToRight,
    /// 把右侧变更块应用到左侧（远端 → 本地）。
    RightToLeft,
}

/// 一次文件差异的完整结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub hunks: Vec<DiffHunk>,
    /// 左侧总行数（按 `\n` 切分的逻辑行数，含末尾空行）。
    pub left_lines: usize,
    /// 右侧总行数。
    pub right_lines: usize,
    /// 右侧相对左侧新增的行数。
    pub additions: usize,
    /// 右侧相对左侧删除的行数。
    pub deletions: usize,
}

impl FileDiff {
    /// 按行计算两侧差异；`context` 为每个变更块前后保留的上下文行数。
    pub fn compute(left: &str, right: &str, context: usize) -> Self {
        let diff = TextDiff::from_lines(left, right);
        let mut hunks = Vec::new();
        let mut additions = 0usize;
        let mut deletions = 0usize;
        for group in diff.grouped_ops(context) {
            let Some(first) = group.first() else {
                continue;
            };
            let Some(last) = group.last() else {
                continue;
            };
            for op in &group {
                match op {
                    DiffOp::Equal { .. } => {}
                    DiffOp::Delete { .. } => deletions += op.old_range().len(),
                    DiffOp::Insert { .. } => additions += op.new_range().len(),
                    DiffOp::Replace { .. } => {
                        deletions += op.old_range().len();
                        additions += op.new_range().len();
                    }
                }
            }
            hunks.push(DiffHunk {
                old_start: first.old_range().start,
                old_len: last.old_range().end.saturating_sub(first.old_range().start),
                new_start: first.new_range().start,
                new_len: last.new_range().end.saturating_sub(first.new_range().start),
            });
        }
        Self {
            hunks,
            left_lines: logical_line_count(left),
            right_lines: logical_line_count(right),
            additions,
            deletions,
        }
    }

    /// 两侧内容是否完全一致。
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// 变更块数量。
    pub fn hunk_count(&self) -> usize {
        self.hunks.len()
    }

    /// 把第 `index` 个变更块从源侧应用到目标侧，返回目标侧合并后的完整文本。
    ///
    /// 只应用一个块；调用方在写回成功后应重新计算差异，块区间会随内容变化而
    /// 重新编号。索引或区间越界返回 `None`（防御来自过期界面的点击）。
    pub fn apply_hunk(
        &self,
        left: &str,
        right: &str,
        index: usize,
        direction: MergeDirection,
    ) -> Option<String> {
        let hunk = self.hunks.get(index)?;
        let (source, target, source_range, target_range) = match direction {
            MergeDirection::LeftToRight => (
                split_lines(left),
                split_lines(right),
                (hunk.old_start, hunk.old_len),
                (hunk.new_start, hunk.new_len),
            ),
            MergeDirection::RightToLeft => (
                split_lines(right),
                split_lines(left),
                (hunk.new_start, hunk.new_len),
                (hunk.old_start, hunk.old_len),
            ),
        };
        let source_end = source_range.0.checked_add(source_range.1)?;
        let target_end = target_range.0.checked_add(target_range.1)?;
        let source_slice = source.get(source_range.0..source_end)?;
        if target_end > target.len() {
            return None;
        }
        let mut merged = target;
        merged.splice(target_range.0..target_end, source_slice.iter().cloned());
        Some(merged.concat())
    }
}

/// 按行切分并保留行尾换行符，与 `similar::TextDiff::from_lines` 的行模型一致。
///
/// 不能用 `split('\n')`：那样 "a\n" 会变成两行，与差异引擎的 1 行不一致，
/// 合并时会在末尾多出一个空元素。`split_inclusive` 让 `concat()` 能无损还原原文。
fn split_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn logical_line_count(text: &str) -> usize {
    text.split_inclusive('\n').count()
}

#[cfg(test)]
mod tests {
    use super::{FileDiff, MergeDirection};

    fn compute(left: &str, right: &str) -> FileDiff {
        // 测试直接校验变更行区间，用 0 上下文避免把相邻等值行并进 hunk。
        FileDiff::compute(left, right, 0)
    }

    #[test]
    fn identical_files_have_no_hunks() {
        let diff = compute("a\nb\n", "a\nb\n");
        assert!(diff.is_empty());
        assert_eq!(diff.additions, 0);
        assert_eq!(diff.deletions, 0);
    }

    #[test]
    fn detects_insert_delete_and_replace() {
        let diff = FileDiff::compute("a\nb\nc\n", "a\nB\nc\nd\n", 3);
        assert_eq!(diff.hunk_count(), 1, "相邻改动应合并成一个块");
        assert_eq!(diff.additions, 2, "替换一行 + 新增一行");
        assert_eq!(diff.deletions, 1);
    }

    #[test]
    fn hunk_ranges_are_zero_based_and_exclusive() {
        let diff = compute("a\nb\nc\n", "a\nx\nc\n");
        let hunk = diff.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_len), (1, 1));
        assert_eq!((hunk.new_start, hunk.new_len), (1, 1));
    }

    #[test]
    fn pure_insertion_has_zero_length_old_range() {
        let diff = compute("a\nc\n", "a\nb\nc\n");
        let hunk = diff.hunks[0];
        assert_eq!(hunk.old_len, 0);
        assert_eq!(hunk.new_len, 1);
    }

    #[test]
    fn applying_left_hunk_to_right_reaches_left_content() {
        let left = "a\nb\nc\n";
        let right = "a\nx\nc\n";
        let diff = compute(left, right);
        let merged = diff
            .apply_hunk(left, right, 0, MergeDirection::LeftToRight)
            .expect("应能应用变更块");
        assert_eq!(merged, left);
    }

    #[test]
    fn applying_right_hunk_to_left_reaches_right_content() {
        let left = "a\nb\nc\n";
        let right = "a\nx\nc\n";
        let diff = compute(left, right);
        let merged = diff
            .apply_hunk(left, right, 0, MergeDirection::RightToLeft)
            .expect("应能反向应用变更块");
        assert_eq!(merged, right);
    }

    #[test]
    fn trailing_newline_is_preserved() {
        let left = "a\n";
        let right = "a";
        let diff = compute(left, right);
        let merged = diff
            .apply_hunk(left, right, 0, MergeDirection::RightToLeft)
            .expect("末尾换行差异也是变更块");
        assert_eq!(merged, "a", "合并结果必须保留目标侧原始末尾状态");
    }

    #[test]
    fn crlf_line_endings_are_preserved() {
        let left = "a\r\nb\r\n";
        let right = "a\r\nx\r\n";
        let diff = compute(left, right);
        let merged = diff
            .apply_hunk(left, right, 0, MergeDirection::LeftToRight)
            .expect("CRLF 文件同样应能合并");
        assert_eq!(merged, left, "合并不得把 CRLF 改成 LF");
    }

    #[test]
    fn multiple_hunks_apply_independently() {
        let left = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let right = "1\nX\n3\n4\n5\n6\n7\n8\nY\n10\n";
        let diff = compute(left, right);
        assert_eq!(diff.hunk_count(), 2, "相隔较远的改动应拆成两个块");
        let first = diff
            .apply_hunk(left, right, 0, MergeDirection::LeftToRight)
            .expect("第一块");
        assert!(first.contains("\n2\n"), "第一块只恢复第 2 行");
        assert!(first.contains("\nY\n"), "第二块此时仍未合并");
    }

    #[test]
    fn out_of_range_hunk_returns_none() {
        let left = "a\n";
        let right = "b\n";
        let diff = compute(left, right);
        assert!(
            diff.apply_hunk(left, right, 99, MergeDirection::LeftToRight)
                .is_none()
        );
    }
}
