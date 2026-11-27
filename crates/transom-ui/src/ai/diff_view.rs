//! 结构化 Unified Diff 逐行着色渲染与长日志输出折叠/关键字高亮组件。
//!
//! 设计原理（Why）：
//! - 将 Diff 解析与日志着色逻辑从 `tool_card.rs` 剥离，控制单文件复杂度并支持审批卡片复用；
//! - 对超长输出默认折叠前部行，仅展示末尾关键行与折叠统计提示，避免窄面板下大日志阻塞滚动。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

const LOG_COLLAPSE_THRESHOLD: usize = 30;
const LOG_TAIL_LINES: usize = 25;

/// Unified Diff 解析后的单行模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DiffLineKind {
    FileHeader,
    HunkHeader,
    Added,
    Removed,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedDiffLine {
    pub(super) kind: DiffLineKind,
    pub(super) old_no: Option<usize>,
    pub(super) new_no: Option<usize>,
    pub(super) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedDiff {
    pub(super) added_count: usize,
    pub(super) removed_count: usize,
    pub(super) lines: Vec<ParsedDiffLine>,
}

/// 日志行语义级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogLineLevel {
    Normal,
    Warning,
    Error,
}

/// 解析 Unified Diff 文本，提取行号与增删行数统计。
pub(super) fn parse_unified_diff(raw: &str) -> ParsedDiff {
    let mut lines = Vec::new();
    let mut added_count = 0usize;
    let mut removed_count = 0usize;
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut in_hunk = false;

    for line in raw.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            lines.push(ParsedDiffLine {
                kind: DiffLineKind::FileHeader,
                old_no: None,
                new_no: None,
                text: line.to_owned(),
            });
            continue;
        }
        if line.starts_with("@@") {
            if let Some((o, n)) = parse_hunk_start(line) {
                old_line = o;
                new_line = n;
                in_hunk = true;
            }
            lines.push(ParsedDiffLine {
                kind: DiffLineKind::HunkHeader,
                old_no: None,
                new_no: None,
                text: line.to_owned(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            added_count += 1;
            let no = in_hunk.then_some(new_line);
            if in_hunk {
                new_line += 1;
            }
            lines.push(ParsedDiffLine {
                kind: DiffLineKind::Added,
                old_no: None,
                new_no: no,
                text: format!("+{rest}"),
            });
        } else if let Some(rest) = line.strip_prefix('-') {
            removed_count += 1;
            let no = in_hunk.then_some(old_line);
            if in_hunk {
                old_line += 1;
            }
            lines.push(ParsedDiffLine {
                kind: DiffLineKind::Removed,
                old_no: no,
                new_no: None,
                text: format!("-{rest}"),
            });
        } else {
            let (o_no, n_no) = if in_hunk {
                let pair = (Some(old_line), Some(new_line));
                old_line += 1;
                new_line += 1;
                pair
            } else {
                (None, None)
            };
            lines.push(ParsedDiffLine {
                kind: DiffLineKind::Context,
                old_no: o_no,
                new_no: n_no,
                text: line.to_owned(),
            });
        }
    }

    ParsedDiff {
        added_count,
        removed_count,
        lines,
    }
}

fn parse_hunk_start(header: &str) -> Option<(usize, usize)> {
    // 形如 `@@ -12,6 +14,8 @@`
    let after_at = header.strip_prefix("@@")?.trim_start();
    let minus_part = after_at.strip_prefix('-')?;
    let old_end = minus_part.find([' ', ','])?;
    let old_start: usize = minus_part[..old_end].parse().ok()?;
    let plus_pos = after_at.find('+')?;
    let plus_part = &after_at[plus_pos + 1..];
    let new_end = plus_part.find([' ', ',', '@'])?;
    let new_start: usize = plus_part[..new_end].parse().ok()?;
    Some((old_start, new_start))
}

/// 识别日志行的告警/错误关键字级别。
pub(super) fn classify_log_line(line: &str) -> LogLineLevel {
    let upper = line.to_ascii_uppercase();
    if upper.contains("ERROR")
        || upper.contains("FATAL")
        || upper.contains("PANIC")
        || upper.contains("FAILED")
        || upper.contains("PERMISSION DENIED")
        || upper.contains("CONNECTION REFUSED")
    {
        LogLineLevel::Error
    } else if upper.contains("WARN")
        || upper.contains("[STDERR]")
        || upper.contains("DEPRECATED")
        || upper.contains("TIMEOUT")
    {
        LogLineLevel::Warning
    } else {
        LogLineLevel::Normal
    }
}

/// 渲染结构化 Unified Diff 视图（含 `+N -M` 变更统计角标、双列行号与红绿背景着色）。
pub(super) fn render_diff_block(raw: &str, p: &Palette) -> AnyElement {
    let parsed = parse_unified_diff(raw);
    let stats_bar = h_flex()
        .w_full()
        .px(rems(space::XS))
        .py(px(3.0))
        .gap(px(8.0))
        .items_center()
        .bg(p.background.opacity(0.6))
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child("变更预览"),
        )
        .child(
            div()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(p.radius.sm)
                .bg(p.success.opacity(0.14))
                .text_size(rems(typography::CAPTION))
                .text_color(p.success)
                .child(format!("+{}", parsed.added_count)),
        )
        .child(
            div()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(p.radius.sm)
                .bg(p.danger.opacity(0.14))
                .text_size(rems(typography::CAPTION))
                .text_color(p.danger)
                .child(format!("-{}", parsed.removed_count)),
        );

    let mut rows = v_flex()
        .w_full()
        .font_family("monospace")
        .text_size(rems(typography::CAPTION));

    for line in parsed.lines {
        let (bg, fg) = match line.kind {
            DiffLineKind::Added => (Some(p.success.opacity(0.13)), p.success),
            DiffLineKind::Removed => (Some(p.danger.opacity(0.13)), p.danger),
            DiffLineKind::HunkHeader => (Some(p.primary.opacity(0.10)), p.primary),
            DiffLineKind::FileHeader => (None, p.text.muted),
            DiffLineKind::Context => (None, p.foreground),
        };
        let old_str = line
            .old_no
            .map(|n| format!("{n:>4}"))
            .unwrap_or_else(|| "    ".to_owned());
        let new_str = line
            .new_no
            .map(|n| format!("{n:>4}"))
            .unwrap_or_else(|| "    ".to_owned());
        let mut row = h_flex()
            .w_full()
            .px(rems(space::XS))
            .py(px(1.0))
            .gap(px(6.0));
        if let Some(bg_col) = bg {
            row = row.bg(bg_col);
        }
        row = row
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(p.text.muted.opacity(0.7))
                    .child(format!("{old_str} {new_str}")),
            )
            .child(div().flex_1().min_w_0().text_color(fg).child(line.text));
        rows = rows.child(row);
    }

    v_flex()
        .w_full()
        .rounded(p.radius.sm)
        .border_1()
        .border_color(p.divider(p.border))
        .overflow_hidden()
        .child(stats_bar)
        .child(rows)
        .into_any_element()
}

/// 渲染命令/工具文本输出（支持超长行折叠摘要与错误/警告关键字高亮）。
pub(super) fn render_log_output(
    id: &str,
    output: &str,
    show_full: bool,
    p: &Palette,
    on_toggle_full: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let all_lines: Vec<&str> = output.lines().collect();
    let total_lines = all_lines.len();
    let need_collapse = total_lines > LOG_COLLAPSE_THRESHOLD;
    let hidden_count = if need_collapse && !show_full {
        total_lines.saturating_sub(LOG_TAIL_LINES)
    } else {
        0
    };
    let visible_slice = &all_lines[hidden_count..];

    let mut box_el = v_flex()
        .w_full()
        .gap(px(1.0))
        .font_family("monospace")
        .text_size(rems(typography::CAPTION));

    if need_collapse {
        let toggle_label = if show_full {
            format!("共 {total_lines} 行 · 点击折叠前部输出")
        } else {
            format!("共 {total_lines} 行（已折叠前 {hidden_count} 行）· 点击展开全部")
        };
        box_el = box_el.child(
            div()
                .id(SharedString::from(format!("ai-log-fold-{id}")))
                .px(px(6.0))
                .py(px(3.0))
                .mb(px(2.0))
                .rounded(p.radius.sm)
                .bg(p.background.opacity(0.6))
                .border_1()
                .border_color(p.divider(p.border))
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .text_color(p.primary)
                .on_click(move |_, w, cx| on_toggle_full(w, cx))
                .child(toggle_label),
        );
    }

    for line in visible_slice {
        let color = match classify_log_line(line) {
            LogLineLevel::Error => p.danger,
            LogLineLevel::Warning => p.warning,
            LogLineLevel::Normal => p.foreground,
        };
        box_el = box_el.child(div().text_color(color).child((*line).to_owned()));
    }

    box_el.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{DiffLineKind, LogLineLevel, classify_log_line, parse_unified_diff};

    #[test]
    fn parses_unified_diff_counts_and_line_numbers() {
        let diff = "\
--- /etc/nginx/nginx.conf
+++ /etc/nginx/nginx.conf
@@ -10,3 +10,4 @@
 worker_processes auto;
-worker_connections 1024;
+worker_connections 4096;
+multi_accept on;
 events {";
        let parsed = parse_unified_diff(diff);
        assert_eq!(parsed.added_count, 2);
        assert_eq!(parsed.removed_count, 1);
        assert_eq!(parsed.lines[0].kind, DiffLineKind::FileHeader);
        assert_eq!(parsed.lines[2].kind, DiffLineKind::HunkHeader);
        assert_eq!(parsed.lines[3].old_no, Some(10));
        assert_eq!(parsed.lines[3].new_no, Some(10));
        assert_eq!(parsed.lines[4].kind, DiffLineKind::Removed);
        assert_eq!(parsed.lines[4].old_no, Some(11));
        assert_eq!(parsed.lines[5].kind, DiffLineKind::Added);
        assert_eq!(parsed.lines[5].new_no, Some(11));
    }

    #[test]
    fn classifies_log_keywords() {
        assert_eq!(
            classify_log_line("2026-11-27 ERROR failed to bind :80"),
            LogLineLevel::Error
        );
        assert_eq!(
            classify_log_line("[stderr] WARNING: low disk space"),
            LogLineLevel::Warning
        );
        assert_eq!(classify_log_line("nginx is running"), LogLineLevel::Normal);
    }
}
