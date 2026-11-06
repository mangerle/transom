//! 差异行计算与行渲染：配对、空白归一、行号槽与词级高亮。
//!
//! 设计原理（Why）：这一段是差异视图里唯一的算法层（`similar` 的结果要按行
//! 配成并排的两列），与工具条菜单拆开后，改对齐口径或改配色只碰一个文件。

use super::{Cell, GUTTER_WIDTH, LINE_HEIGHT, Row, Segment, Tone};
use crate::git::types::{DiffHighlight, GitDiffOptions, IgnoreWhitespace};
use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use similar::{ChangeTag, TextDiff};

/// 计算差异行。
pub(crate) fn build_rows(old: &str, new: &str, options: &GitDiffOptions) -> Vec<Row> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let old_keys: Vec<String> = old_lines
        .iter()
        .map(|line| normalize(line, options.ignore_whitespace))
        .collect();
    let new_keys: Vec<String> = new_lines
        .iter()
        .map(|line| normalize(line, options.ignore_whitespace))
        .collect();
    let old_joined = old_keys.join("\n");
    let new_joined = new_keys.join("\n");
    let diff = TextDiff::from_lines(&old_joined, &new_joined);

    let mut rows = Vec::with_capacity(old_lines.len().min(20_000));
    let mut old_number = 1usize;
    let mut new_number = 1usize;
    let mut pending_removed: Vec<usize> = Vec::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                let index = old_number;
                old_number += 1;
                pending_removed.push(index);
            }
            ChangeTag::Insert => {
                let index = new_number;
                new_number += 1;
                // 连续的删除与插入按行配对，与 IDEA 的并排对齐一致。
                if let Some(front) = pending_removed.first() {
                    rows.push(Row {
                        left: Some(Cell {
                            number: *front,
                            text: line_at(&old_lines, *front),
                        }),
                        right: Some(Cell {
                            number: index,
                            text: line_at(&new_lines, index),
                        }),
                        tone: Tone::Paired,
                    });
                    pending_removed.remove(0);
                } else {
                    rows.push(Row {
                        left: None,
                        right: Some(Cell {
                            number: index,
                            text: line_at(&new_lines, index),
                        }),
                        tone: Tone::Added,
                    });
                }
            }
            ChangeTag::Equal => {
                // 先冲刷掉没配上对的删除行（真正的纯删除）。
                for index in pending_removed.drain(..) {
                    rows.push(Row {
                        left: Some(Cell {
                            number: index,
                            text: line_at(&old_lines, index),
                        }),
                        right: None,
                        tone: Tone::Removed,
                    });
                }
                let (left, right) = (old_number, new_number);
                old_number += 1;
                new_number += 1;
                rows.push(Row {
                    left: Some(Cell {
                        number: left,
                        text: line_at(&old_lines, left),
                    }),
                    right: Some(Cell {
                        number: right,
                        text: line_at(&new_lines, right),
                    }),
                    tone: Tone::Context,
                });
            }
        }
    }
    for index in pending_removed.drain(..) {
        rows.push(Row {
            left: Some(Cell {
                number: index,
                text: line_at(&old_lines, index),
            }),
            right: None,
            tone: Tone::Removed,
        });
    }
    rows
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

fn line_at(lines: &[String], number: usize) -> String {
    lines.get(number - 1).cloned().unwrap_or_default()
}

/// 规范化：只影响匹配，不影响显示；行数保持不变。
fn normalize(line: &str, level: IgnoreWhitespace) -> String {
    match level {
        IgnoreWhitespace::None => line.to_string(),
        IgnoreWhitespace::Trim => line.trim().to_string(),
        IgnoreWhitespace::All => line.split_whitespace().collect::<Vec<_>>().join(" "),
        IgnoreWhitespace::BlankLines => {
            let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if trimmed.is_empty() {
                // 空行统一成一个哨兵值，让"多几个空行"不再算差异。
                "\u{0}".to_string()
            } else {
                trimmed
            }
        }
    }
}

/// 渲染一行。
pub(crate) fn render_row(row: Row, options: &GitDiffOptions, p: &Palette) -> AnyElement {
    // 「高亮 = 无」时不着色，只保留行首标记：有人就是要看结构不看颜色。
    let (bg, marker) = match (row.tone, options.highlight == DiffHighlight::None) {
        (Tone::Added, false) => (p.success.opacity(0.14), "+"),
        (Tone::Removed, false) => (p.danger.opacity(0.14), "-"),
        (Tone::Paired, false) => (p.warning.opacity(0.12), "~"),
        _ => (p.background, " "),
    };
    if !options.side_by_side {
        // 统一视图：只显示有内容的一侧，前缀标出增删。
        let (cell, sign) = match row.tone {
            Tone::Added | Tone::Paired => (row.right.clone(), "+"),
            Tone::Removed => (row.left.clone(), "-"),
            Tone::Context => (row.left.clone(), " "),
        };
        let Some(cell) = cell else {
            return div().into_any_element();
        };
        return h_flex()
            .id(ElementId::Name(
                format!("git-diff-u-{}-{}", cell.number, sign).into(),
            ))
            .h(px(LINE_HEIGHT))
            .items_start()
            .bg(bg)
            .when(!options.wrap_lines, |el| el.truncate())
            .child(gutter(
                options.show_line_numbers.then_some(cell.number.to_string()),
                p,
            ))
            .child(text_line(&cell.text, None, options, p))
            .into_any_element();
    }
    h_flex()
        .id(ElementId::Name(
            format!(
                "git-diff-{}-{}",
                row.left.as_ref().map_or(0, |cell| cell.number),
                row.right.as_ref().map_or(0, |cell| cell.number)
            )
            .into(),
        ))
        .h(px(LINE_HEIGHT))
        .items_start()
        .bg(bg)
        .when(!options.wrap_lines, |el| el.truncate())
        .child(side(
            row.left.as_ref(),
            row.right.as_ref().filter(|_| row.tone == Tone::Paired),
            options,
            p,
        ))
        .child(side(
            row.right.as_ref(),
            row.left.as_ref().filter(|_| row.tone == Tone::Paired),
            options,
            p,
        ))
        .child(
            div()
                .w(px(10.0))
                .text_xs()
                .text_color(p.muted_foreground)
                .child(marker),
        )
        .into_any_element()
}

/// 并排模式的一侧。
///
/// `counterpart` 用于词级高亮：同一行的另一侧文本。
fn side(
    cell: Option<&Cell>,
    counterpart: Option<&Cell>,
    options: &GitDiffOptions,
    p: &Palette,
) -> AnyElement {
    let Some(cell) = cell else {
        return h_flex()
            .flex_1()
            .min_w_0()
            .child(gutter(None, p))
            .child(div().flex_1())
            .bg(p.muted.opacity(0.18))
            .into_any_element();
    };
    h_flex()
        .flex_1()
        .min_w_0()
        .child(gutter(
            options.show_line_numbers.then_some(cell.number.to_string()),
            p,
        ))
        .child(text_line(
            &cell.text,
            counterpart.map(|other| other.text.as_str()),
            options,
            p,
        ))
        .into_any_element()
}

/// 一行文本：按高亮档位切成若干段。
///
/// `counterpart` 是配对行的另一侧文本：只有词级高亮且两侧都存在时才做词比对，
/// 纯增/纯删的行没有可比对象，直接整行由行底色表达。
fn text_line(
    text: &str,
    counterpart: Option<&str>,
    options: &GitDiffOptions,
    p: &Palette,
) -> AnyElement {
    let use_words =
        options.highlight == DiffHighlight::Word && counterpart.is_some() && !text.is_empty();
    if !use_words {
        return div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_xs()
            .font_family(p.mono_font_family.clone())
            .text_color(p.foreground)
            .child(text.to_string())
            .into_any_element();
    }
    let segments = word_segments(text, counterpart.unwrap_or_default());
    h_flex()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .text_xs()
        .font_family(p.mono_font_family.clone())
        .children(segments.into_iter().map(|segment| {
            div()
                .flex_none()
                .px_0p5()
                .text_color(p.foreground)
                .when(segment.changed, |el| el.bg(p.warning.opacity(0.45)))
                .child(segment.text)
        }))
        .into_any_element()
}

/// 把一对行切成「相同 / 已改」的词级分段。
///
/// 两侧各算一份：`TextDiff::from_words` 的删除侧与插入侧天然对应左右两行。
fn word_segments(left: &str, right: &str) -> Vec<Segment> {
    let diff = TextDiff::from_words(left, right);
    let mut segments: Vec<Segment> = Vec::with_capacity(8);
    for change in diff.iter_all_changes() {
        let changed = !matches!(change.tag(), ChangeTag::Equal);
        for value in change.value().split_inclusive(' ') {
            if value.is_empty() {
                continue;
            }
            match segments.last_mut() {
                Some(last) if last.changed == changed => last.text.push_str(value),
                _ => segments.push(Segment {
                    text: value.to_string(),
                    changed,
                }),
            }
        }
    }
    segments
}

fn gutter(number: Option<String>, p: &Palette) -> AnyElement {
    div()
        .w(px(GUTTER_WIDTH))
        .flex_none()
        .text_right()
        .pr_1()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(number.unwrap_or_default())
        .into_any_element()
}
