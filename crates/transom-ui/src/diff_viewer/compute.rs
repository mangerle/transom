//! 差异计算、后台调度与变更块导航。
//!
//! 设计原理（Why）：Myers 计算与统一补丁解析都是 CPU 密集段，放在 UI 线程会
//! 让整个窗口卡顿；这里统一经后台线程执行，并用计算令牌丢弃迟到结果。

use super::{DiffPhase, sanitize_label};
use crate::app::AppShell;
use gpui_kit::component::diff::{DiffFile, DiffLinePosition, DiffMode, DiffSide, DiffState};
use gpui_kit::*;
use similar::TextDiff;
use transom_core::{DEFAULT_CONTEXT_LINES, FileDiff, TransferId};

/// 差异计算产物：一次性地从视图状态里算好，再回到可变状态下安装。
struct PreparedDiff {
    diff: FileDiff,
    files: Vec<DiffFile>,
    path: Option<SharedString>,
}

impl AppShell {
    /// 重算差异并重建 `DiffState`；两侧内容不齐时保持加载态。
    ///
    /// Myers 计算与补丁解析放到后台线程：4 MiB 上限内的文件在慢机器上也
    /// 可能占用数百毫秒，放在 UI 线程会让整个窗口卡住。每次重算生成新的
    /// `compute_token`，迟到的旧结果直接丢弃，避免合并后又被前一次计算覆盖。
    pub(crate) fn recompute_diff(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        let (Some(local), Some(remote)) = (viewer.local_text.clone(), viewer.remote_text.clone())
        else {
            // 另一侧仍在读取：阶段已由读取入口置为 Loading，等回调再触发重算。
            return;
        };
        let left_label = sanitize_label(&format!("a/{}", viewer.local_path.display()));
        let right_label = sanitize_label(&format!("b/{}", viewer.remote_path));
        let language = crate::editor::editor_language_token(&viewer.remote_path);
        let token = TransferId::new();
        viewer.compute_token = token;
        viewer.phase = DiffPhase::Loading;
        viewer.confirm = None;
        cx.spawn(async move |this, cx| {
            let prepared = crate::sftp::spawn_blocking_local(move || {
                prepare_diff(&local, &remote, &left_label, &right_label, &language)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_prepared_diff(token, prepared, cx);
            });
        })
        .detach();
    }

    fn apply_prepared_diff(
        &mut self,
        token: TransferId,
        prepared: Option<PreparedDiff>,
        cx: &mut Context<Self>,
    ) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        if viewer.compute_token != token {
            return;
        }
        let Some(prepared) = prepared else {
            viewer.phase = DiffPhase::Ready;
            viewer.error = Some(rust_i18n::t!("diff.error_compute_spawn").to_string());
            cx.notify();
            return;
        };
        let empty = prepared.diff.is_empty();
        let path = prepared.path.clone();
        let diff = prepared.diff;
        let view = if empty {
            None
        } else {
            Some(cx.new(|cx| DiffState::new(prepared.files, cx).with_mode(DiffMode::Split)))
        };
        if let Some(viewer) = self.diff_viewer.as_mut() {
            if viewer.compute_token != token {
                return;
            }
            viewer.diff = Some(diff);
            viewer.diff_path = path;
            viewer.view = view;
            viewer.hunk_index = (!empty).then_some(0);
            viewer.phase = DiffPhase::Ready;
            viewer.confirm = None;
            viewer.error = None;
            viewer.pending_write = None;
            viewer.pending_read = None;
            viewer.pending_merged = None;
            viewer.pending_local_merged = None;
        }
        if !empty {
            self.scroll_diff_to_hunk(0, cx);
        }
        cx.notify();
    }

    /// 上一处/下一处变更；到端点时循环。
    pub(crate) fn step_diff_hunk(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        let Some(diff) = viewer.diff.as_ref() else {
            return;
        };
        let count = diff.hunks.len();
        if count == 0 {
            return;
        }
        let current = viewer.hunk_index.unwrap_or(0);
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        viewer.hunk_index = Some(next);
        viewer.confirm = None;
        self.scroll_diff_to_hunk(next, cx);
        cx.notify();
    }

    fn scroll_diff_to_hunk(&self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.diff_viewer.as_ref() else {
            return;
        };
        let (Some(view), Some(path), Some(diff)) = (
            viewer.view.as_ref(),
            viewer.diff_path.as_ref(),
            viewer.diff.as_ref(),
        ) else {
            return;
        };
        let Some(hunk) = diff.hunks.get(index) else {
            return;
        };
        let position = DiffLinePosition::new(path.clone(), DiffSide::Modified, hunk.new_start + 1);
        view.update(cx, |state, cx| state.scroll_to_line(position, cx));
    }
}

/// 纯计算段：Myers 行差异 + 统一补丁解析，全部在后台线程执行。
fn prepare_diff(
    local: &str,
    remote: &str,
    left_label: &str,
    right_label: &str,
    language: &str,
) -> PreparedDiff {
    let diff = FileDiff::compute(local, remote, DEFAULT_CONTEXT_LINES);
    if diff.is_empty() {
        return PreparedDiff {
            diff,
            files: Vec::new(),
            path: None,
        };
    }
    let patch = TextDiff::from_lines(local, remote)
        .unified_diff()
        .context_radius(DEFAULT_CONTEXT_LINES)
        .header(left_label, right_label)
        .to_string();
    let parsed = DiffFile::parse(&patch).unwrap_or_default();
    let path = parsed.first().map(|file| file.path().clone());
    let files: Vec<DiffFile> = parsed
        .into_iter()
        .map(|file| file.with_language(language.to_string()))
        .collect();
    PreparedDiff { diff, files, path }
}
