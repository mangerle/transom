//! 编辑面板的未保存变更装饰：行号槽标记与滚动条概览标尺。
//!
//! 设计原理（Why）：线上配置文件的编辑风险主要来自“改了哪些行”不直观。
//! 这里以远端基线为参照计算当前缓冲区差异，并把结果同时投到行号槽和概览标尺；
//! 两个视图共用一次差异结果，避免重复扫描整份文本。
//!
//! 性能与生命周期：输入后等待短暂防抖，再在阻塞线程池计算；每次编辑推进代数，
//! 迟到的旧结果会被丢弃。保存或重新装载基线后会取消任务并清空装饰，防止把
//! 已提交内容继续显示成未保存修改。

use super::entity::EditorPanels;
use crate::palette::Palette;
use gpui_kit::base::input::{GutterDecoration, OverviewRulerDecoration, OverviewRulerLane};
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use similar::{DiffOp, TextDiff};
use std::ops::Range;
use std::time::Duration;

/// 连续输入期间只计算最后一次差异，兼顾反馈速度与后台负载。
const DECORATION_DEBOUNCE: Duration = Duration::from_millis(120);

/// 当前缓冲区相对基线的一种行级变化。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangeKind {
    /// 新增行。
    Insert,
    /// 删除行；在目标文本中以空范围表示行边界。
    Delete,
    /// 替换行。
    Replace,
}

/// 一条待渲染的字节范围及其变化类型。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ChangeRange {
    range: Range<usize>,
    kind: ChangeKind,
}

impl EditorPanels {
    /// 安排一次防抖的未保存变更装饰刷新。
    pub(super) fn schedule_change_decorations(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some((revision, baseline)) = self.map_panel(editor, |item| {
            item.decoration_revision = item.decoration_revision.wrapping_add(1);
            (item.decoration_revision, item.baseline.to_string())
        }) else {
            return;
        };

        let editor_for_snapshot = editor.clone();
        let editor_for_apply = editor.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DECORATION_DEBOUNCE).await;
            let current = this
                .update(cx, |this, cx| {
                    this.map_panel(&editor_for_snapshot, |item| {
                        (item.decoration_revision == revision)
                            .then(|| item.editor.read(cx).value().to_string())
                    })
                    .flatten()
                })
                .ok()
                .flatten();
            let Some(current) = current else {
                return;
            };
            let changed =
                crate::app::spawn_blocking_local(move || changed_line_ranges(&baseline, &current))
                    .await;
            let Some(changed) = changed else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.apply_change_decorations(&editor_for_apply, revision, changed, cx);
            });
        });

        if let Some(item) = self
            .manager_mut()
            .panels
            .iter_mut()
            .find(|item| item.editor == *editor)
        {
            item.decoration_task = Some(task);
        }
    }

    /// 应用仍对应当前代数的差异结果。
    fn apply_change_decorations(
        &mut self,
        editor: &Entity<EditorState>,
        revision: u64,
        changed: Vec<ChangeRange>,
        cx: &mut Context<Self>,
    ) {
        let palette = Palette::from(cx);
        let Some(Some((gutter, overview))) = self.map_panel(editor, |item| {
            if item.decoration_revision != revision {
                return None;
            }
            item.decoration_task = None;
            Some((
                item.gutter_decorations.clone(),
                item.overview_ruler_decorations.clone(),
            ))
        }) else {
            return;
        };

        let gutter_decorations = changed
            .iter()
            .map(|change| {
                let color = change_color(change.kind, &palette);
                GutterDecoration::new(change.range.clone()).with_color(color)
            })
            .collect::<Vec<_>>();
        let overview_decorations = changed
            .iter()
            .map(|change| {
                let color = change_color(change.kind, &palette);
                OverviewRulerDecoration::new(change.range.clone())
                    .with_lane(OverviewRulerLane::Left)
                    .with_color(color)
            })
            .collect::<Vec<_>>();
        gutter.set(gutter_decorations, cx);
        overview.set(overview_decorations, cx);
    }

    /// 基线变化后取消在途计算并清空两套装饰。
    pub(super) fn clear_change_decorations(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let handles = self.map_panel(editor, |item| {
            item.decoration_revision = item.decoration_revision.wrapping_add(1);
            item.decoration_task = None;
            (
                item.gutter_decorations.clone(),
                item.overview_ruler_decorations.clone(),
            )
        });
        if let Some((gutter, overview)) = handles {
            gutter.clear(cx);
            overview.clear(cx);
        }
    }
}

/// 按变化类型选择语义色，避免仅依赖单一强调色表达新增、删除与替换。
fn change_color(kind: ChangeKind, palette: &Palette) -> Hsla {
    match kind {
        ChangeKind::Insert => palette.success,
        ChangeKind::Delete | ChangeKind::Replace => palette.warning,
    }
}

/// 计算当前文本中需要标记的字节范围。
fn changed_line_ranges(baseline: &str, current: &str) -> Vec<ChangeRange> {
    let diff = TextDiff::from_lines(baseline, current);
    let line_starts = line_start_offsets(current);
    let mut changed = Vec::with_capacity(diff.ops().len());
    for op in diff.ops() {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert { .. } => {
                changed.push(ChangeRange {
                    range: line_range(&line_starts, current.len(), op.new_range()),
                    kind: ChangeKind::Insert,
                });
            }
            DiffOp::Delete { .. } => {
                let start = line_offset(&line_starts, current.len(), op.new_range().start);
                changed.push(ChangeRange {
                    range: start..start,
                    kind: ChangeKind::Delete,
                });
            }
            DiffOp::Replace { .. } => {
                changed.push(ChangeRange {
                    range: line_range(&line_starts, current.len(), op.new_range()),
                    kind: ChangeKind::Replace,
                });
            }
        }
    }
    changed
}

fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(text.bytes().filter(|byte| *byte == b'\n').count() + 1);
    starts.push(0);
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' && index + 1 < text.len() {
            starts.push(index + 1);
        }
    }
    starts
}

fn line_offset(starts: &[usize], text_len: usize, line: usize) -> usize {
    starts.get(line).copied().unwrap_or(text_len)
}

fn line_range(starts: &[usize], text_len: usize, lines: Range<usize>) -> Range<usize> {
    let start = line_offset(starts, text_len, lines.start);
    let end = line_offset(starts, text_len, lines.end).max(start);
    start..end
}

#[cfg(test)]
mod tests {
    use super::{ChangeKind, changed_line_ranges};

    #[test]
    fn reports_insert_replace_and_delete_ranges() {
        let baseline = "alpha\nbeta\ngamma\n";
        let current = "alpha\nBETA\ngamma\ndelta\n";
        let changed = changed_line_ranges(baseline, current);
        assert!(
            changed
                .iter()
                .any(|change| change.kind == ChangeKind::Replace)
        );
        assert!(
            changed
                .iter()
                .any(|change| change.kind == ChangeKind::Insert)
        );
    }

    #[test]
    fn deletion_uses_current_text_line_boundary() {
        let changed = changed_line_ranges("a\nb\nc\n", "a\nc\n");
        let deletion = changed
            .iter()
            .find(|change| change.kind == ChangeKind::Delete)
            .expect("应存在删除边界");
        assert!(deletion.range.is_empty(), "删除应使用空范围标记行边界");
    }

    #[test]
    fn identical_text_has_no_decorations() {
        assert!(changed_line_ranges("same\n", "same\n").is_empty());
    }
}
