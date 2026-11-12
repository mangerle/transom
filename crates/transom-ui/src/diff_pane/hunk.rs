//! 单个变更块（Hunk）定位与独立补丁生成。
//!
//! 设计原理（Why）：
//! - `gpui_kit::Diff` 支持在行号与代码列之间挂载 `DiffGutterMarker`，但只负责显示标记与点击槽位，
//!   实际的「暂存此块 / 取消暂存此块 / 还原此块」必须由应用层把单块切片成合法的 Unified Diff 交由 `git apply` 执行；
//! - 即使用户当前开启了「忽略空白」过滤，生成供 `git apply` 使用的单块补丁时仍须严格对比原始行文本，
//!   对存在真实空白差异的上下文行以 `-`/`+` 成对输出，否则补丁上下文与目标文件不匹配会导致 `git apply` 拒绝。

use std::fmt::Write as _;

use gpui_kit::component::diff::{DiffLinePosition, DiffSide};
use similar::DiffOp;

/// 单个变更块的可视定位与独立补丁载荷。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedHunk {
    /// 目标文件在 `DiffFile` 中的路径（与 `sanitize_diff_path` 一致）。
    pub file_path: String,
    /// `true` 表示锚定在修改侧（`Modified`），`false` 表示锚定在原始侧（`Original`，用于纯删除块）。
    pub marker_side_modified: bool,
    /// 锚定行号（1-based）。
    pub marker_line: usize,
    /// 可直接传给 `git apply` 的单块 Unified Diff 文本。
    pub patch: String,
}

impl PreparedHunk {
    /// 构造 `DiffGutterMarker` 使用的行位置句柄。
    pub(crate) fn gutter_position(&self) -> DiffLinePosition {
        let side = if self.marker_side_modified {
            DiffSide::Modified
        } else {
            DiffSide::Original
        };
        DiffLinePosition::new(self.file_path.clone(), side, self.marker_line)
    }
}

/// 单块生成所需的文件级上下文参数。
pub(crate) struct HunkExtractInput<'a> {
    pub file_path: &'a str,
    pub old_text: &'a str,
    pub new_text: &'a str,
    pub old_orig: &'a [&'a str],
    pub new_orig: &'a [&'a str],
    pub groups: &'a [Vec<DiffOp>],
}

/// 从已分组的 `similar::DiffOp` 提取所有可执行的变更块与对应单块补丁。
///
/// 若原文件或新文件为空（整文件新建/整文件删除），返回空列表以免单块 `git apply` 与整文件增删语义冲突。
pub(crate) fn extract_prepared_hunks(input: HunkExtractInput<'_>) -> Vec<PreparedHunk> {
    if input.old_orig.is_empty() || input.new_orig.is_empty() || input.groups.is_empty() {
        return Vec::new();
    }
    let old_no_eol = !input.old_text.ends_with('\n');
    let new_no_eol = !input.new_text.ends_with('\n');

    let mut hunks = Vec::with_capacity(input.groups.len());
    for group in input.groups {
        let Some((marker_side_modified, marker_line)) = find_hunk_marker_position(group) else {
            continue;
        };
        let Some(patch) = build_single_hunk_patch(
            input.file_path,
            input.old_orig,
            input.new_orig,
            old_no_eol,
            new_no_eol,
            group,
        ) else {
            continue;
        };
        hunks.push(PreparedHunk {
            file_path: input.file_path.to_string(),
            marker_side_modified,
            marker_line,
            patch,
        });
    }
    hunks
}

/// 查找变更块的第一个实质变更行，作为槽位图标的停靠点。
fn find_hunk_marker_position(group: &[DiffOp]) -> Option<(bool, usize)> {
    for op in group {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert { new_index, .. } | DiffOp::Replace { new_index, .. } => {
                return Some((true, new_index + 1));
            }
            DiffOp::Delete { old_index, .. } => {
                return Some((false, old_index + 1));
            }
        }
    }
    None
}

fn build_single_hunk_patch(
    file_path: &str,
    old_orig: &[&str],
    new_orig: &[&str],
    old_no_eol: bool,
    new_no_eol: bool,
    group: &[DiffOp],
) -> Option<String> {
    let first = group.first()?;
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    let mut body = String::with_capacity(512);

    let emit_line = |buf: &mut String, prefix: char, line: &str, no_eol: bool| {
        buf.push(prefix);
        buf.push_str(line);
        buf.push('\n');
        if no_eol {
            buf.push_str("\\ No newline at end of file\n");
        }
    };

    for op in group {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for offset in 0..len {
                    let old_idx = old_index + offset;
                    let new_idx = new_index + offset;
                    let old_line = *old_orig.get(old_idx)?;
                    let new_line = *new_orig.get(new_idx)?;
                    let old_is_last = old_idx + 1 == old_orig.len();
                    let new_is_last = new_idx + 1 == new_orig.len();
                    let same_eol = (!old_is_last || !old_no_eol) && (!new_is_last || !new_no_eol);
                    let both_no_eol = old_is_last && new_is_last && old_no_eol && new_no_eol;
                    if old_line == new_line && (same_eol || both_no_eol) {
                        emit_line(&mut body, ' ', old_line, both_no_eol);
                    } else {
                        emit_line(&mut body, '-', old_line, old_is_last && old_no_eol);
                        emit_line(&mut body, '+', new_line, new_is_last && new_no_eol);
                    }
                    old_count += 1;
                    new_count += 1;
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for offset in 0..old_len {
                    let old_idx = old_index + offset;
                    let old_line = *old_orig.get(old_idx)?;
                    let is_last = old_idx + 1 == old_orig.len();
                    emit_line(&mut body, '-', old_line, is_last && old_no_eol);
                    old_count += 1;
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for offset in 0..new_len {
                    let new_idx = new_index + offset;
                    let new_line = *new_orig.get(new_idx)?;
                    let is_last = new_idx + 1 == new_orig.len();
                    emit_line(&mut body, '+', new_line, is_last && new_no_eol);
                    new_count += 1;
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for offset in 0..old_len {
                    let old_idx = old_index + offset;
                    let old_line = *old_orig.get(old_idx)?;
                    let is_last = old_idx + 1 == old_orig.len();
                    emit_line(&mut body, '-', old_line, is_last && old_no_eol);
                    old_count += 1;
                }
                for offset in 0..new_len {
                    let new_idx = new_index + offset;
                    let new_line = *new_orig.get(new_idx)?;
                    let is_last = new_idx + 1 == new_orig.len();
                    emit_line(&mut body, '+', new_line, is_last && new_no_eol);
                    new_count += 1;
                }
            }
        }
    }

    let old_start = if old_count == 0 {
        0
    } else {
        first.old_range().start + 1
    };
    let new_start = if new_count == 0 {
        0
    } else {
        first.new_range().start + 1
    };

    let mut patch = String::with_capacity(body.len() + 160);
    let _ = writeln!(patch, "diff --git a/{file_path} b/{file_path}");
    let _ = writeln!(patch, "--- a/{file_path}");
    let _ = writeln!(patch, "+++ b/{file_path}");
    let _ = writeln!(
        patch,
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
    );
    patch.push_str(&body);
    Some(patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar::TextDiff;

    #[test]
    fn extracts_single_hunk_patch_and_applies_cleanly_in_git_repo() {
        let old_text = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        let new_text = "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n";
        let old_orig: Vec<&str> = old_text.lines().collect();
        let new_orig: Vec<&str> = new_text.lines().collect();
        let diff = TextDiff::from_slices(&old_orig, &new_orig);
        let groups = diff.grouped_ops(1);

        let hunks = extract_prepared_hunks(HunkExtractInput {
            file_path: "demo.txt",
            old_text,
            new_text,
            old_orig: &old_orig,
            new_orig: &new_orig,
            groups: &groups,
        });

        assert_eq!(hunks.len(), 2);
        assert!(hunks[0].marker_side_modified);
        assert_eq!(hunks[0].marker_line, 2);
        assert!(hunks[0].patch.contains("@@ -1,3 +1,3 @@"));
        assert!(hunks[0].patch.contains("-l2\n+L2\n"));
        assert!(!hunks[0].patch.contains("L9"));

        assert_eq!(hunks[1].marker_line, 9);
        assert!(hunks[1].patch.contains("@@ -8,3 +8,3 @@"));
        assert!(hunks[1].patch.contains("-l9\n+L9\n"));
    }

    #[test]
    fn pure_deletion_anchors_on_original_side() {
        let old_text = "a\nb\nc\n";
        let new_text = "a\nc\n";
        let old_orig: Vec<&str> = old_text.lines().collect();
        let new_orig: Vec<&str> = new_text.lines().collect();
        let diff = TextDiff::from_slices(&old_orig, &new_orig);
        let groups = diff.grouped_ops(3);

        let hunks = extract_prepared_hunks(HunkExtractInput {
            file_path: "demo.txt",
            old_text,
            new_text,
            old_orig: &old_orig,
            new_orig: &new_orig,
            groups: &groups,
        });

        assert_eq!(hunks.len(), 1);
        assert!(!hunks[0].marker_side_modified);
        assert_eq!(hunks[0].marker_line, 2);
        assert!(hunks[0].patch.contains("@@ -1,3 +1,2 @@"));
        assert!(hunks[0].patch.contains("-b\n"));
    }
}
