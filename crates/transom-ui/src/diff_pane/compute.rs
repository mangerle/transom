//! 公共 Diff 计算引擎：后台 Myers 比对、空白忽略、全量上下文补丁生成与状态同步。
//!
//! 设计原理（Why）：
//! - 组件库 `DiffState` 接收 `DiffFile::parse(&patch)` 的产物并原生提供语法高亮、
//!   划词复制、并排/统一布局以及未变更行折叠展开能力；
//! - 为了既支持 4 档 `IgnoreWhitespace` 过滤，又保留源代码的原始缩进与文字，
//!   我们先对规范化后的行做 Myers 比对，再按原行文本构造覆盖全文件的 unified patch：
//!   被忽略的空白差异行会作为上下文原样输出，不会产生冗余红绿块，且因为传入了
//!   完整上下文，`DiffState` 的折叠与局部展开功能（Up/Down/All）得以开箱即用；
//! - 所有计算都在纯函数中完成，由调用方放到 `spawn_blocking_local` 后台线程执行，
//!   避免大文件在 UI 线程卡死主事件循环。

use std::fmt::Write as _;

use gpui_kit::component::diff::{DiffFile, DiffInlineUnit, DiffMode, DiffState};
use gpui_kit::{App, AppContext as _, Entity};
use similar::{ChangeTag, DiffOp, TextDiff};

use super::hunk::{HunkExtractInput, extract_prepared_hunks};
use super::types::{DiffHighlight, DiffViewOptions, IgnoreWhitespace, PreparedDiffDocument};

/// 默认折叠状态下每个变更块周围保留的上下文行数。
pub(crate) const DEFAULT_DIFF_CONTEXT_LINES: usize = 3;

/// 按行切分文本并剥离行尾 `\r`，用于补丁文本拼接与空白规范化。
pub(crate) fn split_lines_stripped(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    // 使用 `split('\n')` 处理：若末尾以 `\n` 结尾，最后一个空串代表终止换行，不计作正文行。
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// 按空白忽略档位生成比对键；仅用于匹配，不修改显示文本。
pub(crate) fn normalize_line(line: &str, level: IgnoreWhitespace) -> String {
    match level {
        IgnoreWhitespace::None => line.to_string(),
        IgnoreWhitespace::Trim => line.trim().to_string(),
        IgnoreWhitespace::All => line.split_whitespace().collect::<Vec<_>>().join(" "),
        IgnoreWhitespace::BlankLines => {
            let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if collapsed.is_empty() {
                // 空行统一映射为哨兵字符，使连续空行增减不触发差异。
                "\u{0}".to_string()
            } else {
                collapsed
            }
        }
    }
}

/// 清洗补丁头部路径：去除换行/制表符，避免破坏 `---`/`+++` 解析格式。
pub(crate) fn sanitize_diff_path(path: &str) -> String {
    let cleaned = path.replace(['\n', '\r', '\t'], " ");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "file".to_string()
    } else if trimmed.starts_with('"') {
        trimmed.trim_start_matches('"').to_string()
    } else {
        trimmed.to_string()
    }
}

/// 纯计算函数：在后台线程比对左右文本并生成供 `DiffState` 消费的文档。
///
/// 即使启用了空白忽略，最终展示的文本仍为左右两侧的原始行。
pub(crate) fn prepare_diff_document(
    old: &str,
    new: &str,
    path: &str,
    language: &str,
    ignore_whitespace: IgnoreWhitespace,
) -> PreparedDiffDocument {
    let old_orig = split_lines_stripped(old);
    let new_orig = split_lines_stripped(new);

    let (old_owned, new_owned): (Vec<String>, Vec<String>) =
        if ignore_whitespace == IgnoreWhitespace::None {
            (Vec::new(), Vec::new())
        } else {
            (
                old_orig
                    .iter()
                    .map(|line| normalize_line(line, ignore_whitespace))
                    .collect(),
                new_orig
                    .iter()
                    .map(|line| normalize_line(line, ignore_whitespace))
                    .collect(),
            )
        };

    let (old_keys, new_keys): (Vec<&str>, Vec<&str>) =
        if ignore_whitespace == IgnoreWhitespace::None {
            (old_orig.clone(), new_orig.clone())
        } else {
            (
                old_owned.iter().map(String::as_str).collect(),
                new_owned.iter().map(String::as_str).collect(),
            )
        };

    if ignore_whitespace == IgnoreWhitespace::BlankLines {
        let old_non_empty: Vec<&str> = old_keys
            .iter()
            .copied()
            .filter(|line| *line != "\u{0}")
            .collect();
        let new_non_empty: Vec<&str> = new_keys
            .iter()
            .copied()
            .filter(|line| *line != "\u{0}")
            .collect();
        if old_non_empty == new_non_empty {
            return PreparedDiffDocument {
                files: Vec::new(),
                additions: 0,
                deletions: 0,
                hunk_count: 0,
                hunks: Vec::new(),
            };
        }
    }

    // 直接基于行切片比对，避免 `join("\n")` 丢失末尾空行或导致末尾行换行符不一致。
    let diff = TextDiff::from_slices(&old_keys, &new_keys);

    let groups = diff.grouped_ops(DEFAULT_DIFF_CONTEXT_LINES);
    let hunk_count = groups.len();
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for group in &groups {
        for op in group {
            match op {
                DiffOp::Equal { .. } => {}
                DiffOp::Delete { old_len, .. } => deletions += *old_len,
                DiffOp::Insert { new_len, .. } => additions += *new_len,
                DiffOp::Replace {
                    old_len, new_len, ..
                } => {
                    deletions += *old_len;
                    additions += *new_len;
                }
            }
        }
    }

    if hunk_count == 0 || (additions == 0 && deletions == 0) {
        return PreparedDiffDocument {
            files: Vec::new(),
            additions: 0,
            deletions: 0,
            hunk_count: 0,
            hunks: Vec::new(),
        };
    }

    let clean_path = sanitize_diff_path(path);
    let hunks = extract_prepared_hunks(HunkExtractInput {
        file_path: &clean_path,
        old_text: old,
        new_text: new,
        old_orig: &old_orig,
        new_orig: &new_orig,
        groups: &groups,
    });
    let patch = build_full_context_patch(path, &old_orig, &new_orig, &diff);
    let parsed = DiffFile::parse(&patch).unwrap_or_else(|error| {
        log::error!("构造内部差异补丁解析失败，路径: {path}，原因: {error:?}");
        Vec::new()
    });
    let files: Vec<DiffFile> = parsed
        .into_iter()
        .map(|file| file.with_language(language.to_string()))
        .collect();

    PreparedDiffDocument {
        files,
        additions,
        deletions,
        hunk_count,
        hunks,
    }
}

/// 生成包含完整文件行上下文的 unified patch，确保 `DiffState` 可自行折叠与局部展开。
fn build_full_context_patch<'a>(
    path: &str,
    old_orig: &[&str],
    new_orig: &[&str],
    diff: &TextDiff<'a, 'a, 'a, str>,
) -> String {
    let clean_path = sanitize_diff_path(path);
    let old_count = old_orig.len();
    let new_count = new_orig.len();
    let old_start = usize::from(old_count > 0);
    let new_start = usize::from(new_count > 0);

    let estimated = old_orig.iter().map(|l| l.len() + 2).sum::<usize>()
        + new_orig.iter().map(|l| l.len() + 2).sum::<usize>()
        + 128;
    let mut patch = String::with_capacity(estimated);
    if old_count == 0 {
        let _ = writeln!(patch, "--- /dev/null");
    } else {
        let _ = writeln!(patch, "--- a/{clean_path}");
    }
    if new_count == 0 {
        let _ = writeln!(patch, "+++ /dev/null");
    } else {
        let _ = writeln!(patch, "+++ b/{clean_path}");
    }
    let _ = writeln!(
        patch,
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
    );

    let mut old_cursor = 0usize;
    let mut new_cursor = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                // 使用修改侧原文作为上下文（在未忽略空白时两侧一致；忽略空白时展示当前版本）。
                let text = new_orig
                    .get(new_cursor)
                    .or_else(|| old_orig.get(old_cursor))
                    .copied()
                    .unwrap_or_default();
                patch.push(' ');
                patch.push_str(text);
                patch.push('\n');
                old_cursor += 1;
                new_cursor += 1;
            }
            ChangeTag::Delete => {
                let text = old_orig.get(old_cursor).copied().unwrap_or_default();
                patch.push('-');
                patch.push_str(text);
                patch.push('\n');
                old_cursor += 1;
            }
            ChangeTag::Insert => {
                let text = new_orig.get(new_cursor).copied().unwrap_or_default();
                patch.push('+');
                patch.push_str(text);
                patch.push('\n');
                new_cursor += 1;
            }
        }
    }
    patch
}

/// 根据已准备好的文档和显示选项构建 `DiffState` 实体。
pub(crate) fn create_diff_state(
    files: Vec<DiffFile>,
    options: DiffViewOptions,
    cx: &mut App,
) -> Entity<DiffState> {
    let mode = if options.side_by_side {
        DiffMode::Split
    } else {
        DiffMode::Unified
    };
    let context = options
        .collapse_unchanged
        .then_some(DEFAULT_DIFF_CONTEXT_LINES);
    let inline_unit = (options.highlight == DiffHighlight::Word).then_some(DiffInlineUnit::Word);
    cx.new(|cx| {
        DiffState::new(files, cx)
            .with_mode(mode)
            .with_context_lines(context)
            .with_inline_unit(inline_unit)
    })
}

/// 将不涉及重新比对的显示选项增量同步到现有 `DiffState`（保留滚动位置与文本选区）。
pub(crate) fn sync_diff_state_options(
    diff_state: &Entity<DiffState>,
    options: DiffViewOptions,
    cx: &mut App,
) {
    let mode = if options.side_by_side {
        DiffMode::Split
    } else {
        DiffMode::Unified
    };
    let context = options
        .collapse_unchanged
        .then_some(DEFAULT_DIFF_CONTEXT_LINES);
    let inline_unit = (options.highlight == DiffHighlight::Word).then_some(DiffInlineUnit::Word);
    diff_state.update(cx, |state, cx| {
        if state.mode() != mode {
            state.set_mode(mode, cx);
        }
        if state.context_lines() != context {
            state.set_context_lines(context, cx);
        }
        if state.inline_unit() != inline_unit {
            state.set_inline_unit(inline_unit, cx);
        }
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::diff::DiffFileStatus;

    use super::{prepare_diff_document, split_lines_stripped};
    use crate::diff_pane::types::IgnoreWhitespace;

    #[test]
    fn split_lines_handles_empty_and_trailing_newline() {
        assert!(split_lines_stripped("").is_empty());
        assert_eq!(split_lines_stripped("a\nb\n"), vec!["a", "b"]);
        assert_eq!(split_lines_stripped("a\r\nb"), vec!["a", "b"]);
    }

    #[test]
    fn prepare_detects_no_diff_for_identical_text() {
        let doc = prepare_diff_document(
            "fn main() {}\n",
            "fn main() {}\n",
            "src/main.rs",
            "rs",
            IgnoreWhitespace::None,
        );
        assert!(doc.is_empty());
        assert_eq!(doc.hunk_count, 0);
    }

    #[test]
    fn prepare_builds_valid_diff_for_added_deleted_and_modified_files() {
        let added = prepare_diff_document("", "line1\nline2\n", "a.rs", "rs", IgnoreWhitespace::None);
        assert!(!added.is_empty());
        assert_eq!(added.additions, 2);
        assert_eq!(added.deletions, 0);
        assert_eq!(added.files.len(), 1);
        assert_eq!(added.files[0].status(), DiffFileStatus::Added);
        assert_eq!(added.files[0].additions(), 2);

        let deleted =
            prepare_diff_document("line1\nline2\n", "", "a.rs", "rs", IgnoreWhitespace::None);
        assert!(!deleted.is_empty());
        assert_eq!(deleted.additions, 0);
        assert_eq!(deleted.deletions, 2);
        assert_eq!(deleted.files.len(), 1);
        assert_eq!(deleted.files[0].status(), DiffFileStatus::Deleted);
        assert_eq!(deleted.files[0].deletions(), 2);
    }

    #[test]
    fn append_or_delete_trailing_line_does_not_mark_previous_line_changed() {
        let appended =
            prepare_diff_document("a\nb\n", "a\nb\nc\n", "a.txt", "txt", IgnoreWhitespace::None);
        assert_eq!(appended.additions, 1);
        assert_eq!(appended.deletions, 0);
        assert_eq!(appended.files[0].additions(), 1);
        assert_eq!(appended.files[0].deletions(), 0);

        let appended_no_eol =
            prepare_diff_document("a\nb", "a\nb\nc", "a.txt", "txt", IgnoreWhitespace::None);
        assert_eq!(appended_no_eol.additions, 1);
        assert_eq!(appended_no_eol.deletions, 0);

        let removed =
            prepare_diff_document("a\nb\nc\n", "a\nb\n", "a.txt", "txt", IgnoreWhitespace::None);
        assert_eq!(removed.additions, 0);
        assert_eq!(removed.deletions, 1);
    }

    #[test]
    fn single_blank_line_file_parses_cleanly() {
        let modified =
            prepare_diff_document("\n", "hello\n", "empty.txt", "txt", IgnoreWhitespace::None);
        assert_eq!(modified.files.len(), 1);
        assert_eq!(modified.deletions, 1);
        assert_eq!(modified.additions, 1);

        let cleared =
            prepare_diff_document("\n", "", "empty.txt", "txt", IgnoreWhitespace::None);
        assert_eq!(cleared.files.len(), 1);
        assert_eq!(cleared.deletions, 1);
        assert_eq!(cleared.additions, 0);
    }

    #[test]
    fn ignore_whitespace_modes_filter_expected_changes_and_preserve_source_lines() {
        let old = "  let x = 1;\n\nlet y = 2;\n";
        let new = "let x = 1;\n\n\nlet   y = 2;\n";

        let none = prepare_diff_document(old, new, "a.rs", "rs", IgnoreWhitespace::None);
        assert!(!none.is_empty());

        let trim = prepare_diff_document(old, new, "a.rs", "rs", IgnoreWhitespace::Trim);
        // Trim 忽略行首空白差异，但仍保留中间多个空格和空行差异。
        assert!(!trim.is_empty());

        let all = prepare_diff_document(old, new, "a.rs", "rs", IgnoreWhitespace::All);
        // All 忽略 `let y` 与 `let   y`，只剩空行差异。
        assert_eq!(all.additions, 1);
        assert_eq!(all.deletions, 0);

        let blank = prepare_diff_document(old, new, "a.rs", "rs", IgnoreWhitespace::BlankLines);
        assert!(blank.is_empty(), "BlankLines 模式下不应有剩余差异");
    }

    #[test]
    fn prepare_retains_full_context_for_collapsible_hunks() {
        let old = (1..=30).map(|i| format!("line {i}\n")).collect::<String>();
        let mut new_lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        new_lines[14] = "changed 15".to_string();
        let new = format!("{}\n", new_lines.join("\n"));

        let doc = prepare_diff_document(&old, &new, "notes.txt", "txt", IgnoreWhitespace::None);
        assert_eq!(doc.hunk_count, 1);
        assert_eq!(doc.files.len(), 1);
        assert_eq!(doc.files[0].additions(), 1);
        assert_eq!(doc.files[0].deletions(), 1);

        let old_orig = split_lines_stripped(&old);
        let new_orig = split_lines_stripped(&new);
        let diff = similar::TextDiff::from_slices(&old_orig, &new_orig);
        let patch = super::build_full_context_patch("notes.txt", &old_orig, &new_orig, &diff);
        assert!(patch.contains("@@ -1,30 +1,30 @@"));
        assert!(patch.contains(" line 1\n"));
        assert!(patch.contains(" line 30\n"));
    }
}
