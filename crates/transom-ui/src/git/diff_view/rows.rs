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
    if !options.side_by_side {
        // 统一视图（Unified Diff）：修改行按标准双行显示（先删后增并互相词级比对）。
        if row.tone == Tone::Paired {
            let left_cell = row.left.as_ref();
            let right_cell = row.right.as_ref();
            let del_bg = p.danger.opacity(0.12);
            let del_word_bg = p.danger.opacity(0.30);
            let add_bg = p.success.opacity(0.12);
            let add_word_bg = p.success.opacity(0.30);

            let mut group = gpui_kit::base::v_flex().w_full();
            if let Some(left) = left_cell {
                group = group.child(
                    h_flex()
                        .id(ElementId::Name(
                            format!("git-diff-u-{}-del", left.number).into(),
                        ))
                        .w_full()
                        .h(px(LINE_HEIGHT))
                        .items_center()
                        .bg(del_bg)
                        .when(!options.wrap_lines, |el| el.truncate())
                        .child(gutter(
                            options.show_line_numbers.then_some(left.number.to_string()),
                            p,
                        ))
                        .child(text_line(
                            &left.text,
                            right_cell.map(|r| r.text.as_str()),
                            options,
                            del_word_bg,
                            p,
                        )),
                );
            }
            if let Some(right) = right_cell {
                group = group.child(
                    h_flex()
                        .id(ElementId::Name(
                            format!("git-diff-u-{}-add", right.number).into(),
                        ))
                        .w_full()
                        .h(px(LINE_HEIGHT))
                        .items_center()
                        .bg(add_bg)
                        .when(!options.wrap_lines, |el| el.truncate())
                        .child(gutter(
                            options
                                .show_line_numbers
                                .then_some(right.number.to_string()),
                            p,
                        ))
                        .child(text_line(
                            &right.text,
                            left_cell.map(|l| l.text.as_str()),
                            options,
                            add_word_bg,
                            p,
                        )),
                );
            }
            return group.into_any_element();
        }

        // 纯新增、纯删除或上下文行
        let (cell, sign, row_bg, row_word_bg) = match row.tone {
            Tone::Added => (
                row.right.clone(),
                "+",
                p.success.opacity(0.12),
                p.success.opacity(0.30),
            ),
            Tone::Removed => (
                row.left.clone(),
                "-",
                p.danger.opacity(0.12),
                p.danger.opacity(0.30),
            ),
            _ => (
                row.right.or(row.left),
                " ",
                p.background,
                p.primary.opacity(0.30),
            ),
        };
        let Some(cell) = cell else {
            return div().into_any_element();
        };
        return h_flex()
            .id(ElementId::Name(
                format!("git-diff-u-{}-{}", cell.number, sign).into(),
            ))
            .w_full()
            .h(px(LINE_HEIGHT))
            .items_center()
            .bg(row_bg)
            .when(!options.wrap_lines, |el| el.truncate())
            .child(gutter(
                options.show_line_numbers.then_some(cell.number.to_string()),
                p,
            ))
            .child(text_line(&cell.text, None, options, row_word_bg, p))
            .into_any_element();
    }

    let left_side = side(
        row.left.as_ref(),
        row.right.as_ref().filter(|_| row.tone == Tone::Paired),
        options,
        row.tone,
        false,
        p,
    );
    let right_side = side(
        row.right.as_ref(),
        row.left.as_ref().filter(|_| row.tone == Tone::Paired),
        options,
        row.tone,
        true,
        p,
    );

    h_flex()
        .id(ElementId::Name(
            format!(
                "git-diff-{}-{}",
                row.left.as_ref().map_or(0, |cell| cell.number),
                row.right.as_ref().map_or(0, |cell| cell.number)
            )
            .into(),
        ))
        .w_full()
        .h(px(LINE_HEIGHT))
        .items_center()
        .when(!options.wrap_lines, |el| el.truncate())
        .child(left_side)
        .child(right_side)
        .into_any_element()
}

/// 并排模式的一侧。
///
/// `counterpart` 用于词级高亮：同一行的另一侧文本。
fn side(
    cell: Option<&Cell>,
    counterpart: Option<&Cell>,
    options: &GitDiffOptions,
    tone: Tone,
    is_right: bool,
    p: &Palette,
) -> AnyElement {
    let (side_bg, word_bg) = match (cell.is_some(), tone) {
        (false, _) => (p.muted.opacity(0.12), p.primary.opacity(0.30)),
        (true, Tone::Added) => (p.success.opacity(0.14), p.success.opacity(0.32)),
        (true, Tone::Removed) => (p.danger.opacity(0.14), p.danger.opacity(0.32)),
        (true, Tone::Paired) => {
            if is_right {
                (p.success.opacity(0.12), p.success.opacity(0.30))
            } else {
                (p.danger.opacity(0.12), p.danger.opacity(0.30))
            }
        }
        (true, Tone::Context) => (p.background, p.primary.opacity(0.30)),
    };

    let base = h_flex()
        .w_1_2()
        .h_full()
        .min_w_0()
        .items_center()
        .bg(side_bg)
        .when(is_right, |el| {
            el.border_color(p.divider(p.border)).border_l_1()
        });

    let Some(cell) = cell else {
        return base
            .child(gutter(None, p))
            .child(div().flex_1())
            .into_any_element();
    };

    base.child(gutter(
        options.show_line_numbers.then_some(cell.number.to_string()),
        p,
    ))
    .child(text_line(
        &cell.text,
        counterpart.map(|other| other.text.as_str()),
        options,
        word_bg,
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
    word_bg: Hsla,
    p: &Palette,
) -> AnyElement {
    let use_words =
        options.highlight == DiffHighlight::Word && counterpart.is_some() && !text.is_empty();
    if !use_words {
        return div()
            .flex_1()
            .min_w_0()
            .pl_2()
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
        .pl_2()
        .overflow_hidden()
        .text_xs()
        .font_family(p.mono_font_family.clone())
        .children(segments.into_iter().map(|segment| {
            div()
                .flex_none()
                .px_0p5()
                .rounded(px(2.0))
                .text_color(p.foreground)
                .when(segment.changed, |el| el.bg(word_bg))
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
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .pr_2()
        .text_xs()
        .font_family(p.mono_font_family.clone())
        .text_color(p.muted_foreground.opacity(0.55))
        .child(number.unwrap_or_default())
        .into_any_element()
}
