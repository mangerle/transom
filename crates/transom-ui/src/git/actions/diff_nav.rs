//! Git 差异跨文件块导航、主动/自动刷新与源码跳转打开动作。
//!
//! 设计原理（Why）：
//! - 将跨文件差异步进、`StatusLoaded` 自动重载及工具栏刷新/打开源文件等逻辑从
//!   `diff_actions.rs` 拆分，避免单文件膨胀超过 500 行阈值；
//! - 跨文件导航以标签页内 `current_hunk` 游标与目标页 `pending_edge_jump` 为状态载体，
//!   既能平滑跨越尚未计算完的标签边界，又不污染渲染层状态。

use std::path::Path;

use super::diff_actions::diff_tab_needs_recompute;
use super::diff_search::recompute_workspace_diff_search;
use crate::audit::open_local_path;
use crate::git::GitWorkspace;
use crate::git::types::GitDiffState;
use gpui_kit::component::diff::{DiffLinePosition, DiffState};
use gpui_kit::{Context, Entity};
use transom_core::{GitCommand, GitRequestId, GitSide};

/// 差异标签批量关闭范围（对应标签栏右键菜单与工具栏更多菜单）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffTabCloseScope {
    /// 关闭除锚点之外的全部标签。
    Other,
    /// 关闭锚点左侧所有标签。
    Left,
    /// 关闭锚点右侧所有标签。
    Right,
    /// 关闭全部标签。
    All,
}

enum StepOutcome {
    None,
    WrapSingleState(Entity<DiffState>),
    JumpInCurrent(Entity<DiffState>, DiffLinePosition),
    SwitchTab(usize),
}

fn next_hunk_index(current: Option<usize>, len: usize, forward: bool) -> Result<usize, bool> {
    if forward {
        match current {
            None => Ok(0),
            Some(cur) if cur + 1 < len => Ok(cur + 1),
            Some(_) => Err(true),
        }
    } else {
        match current {
            None => Ok(len - 1),
            Some(cur) if cur > 0 => Ok(cur - 1),
            Some(_) => Err(false),
        }
    }
}

impl GitWorkspace {
    /// 上一处/下一处差异：到达当前文件首尾边界时自动切入上/下一差异标签。
    pub(crate) fn step_diff_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let outcome = self.compute_step_outcome(forward);
        match outcome {
            StepOutcome::None => {}
            StepOutcome::WrapSingleState(ds) => {
                ds.update(cx, |state, cx| {
                    if forward {
                        state.next_change(cx);
                    } else {
                        state.previous_change(cx);
                    }
                });
                cx.notify();
            }
            StepOutcome::JumpInCurrent(ds, pos) => {
                ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
                cx.notify();
            }
            StepOutcome::SwitchTab(target_idx) => {
                self.select_diff_tab(target_idx, cx);
            }
        }
    }

    fn compute_step_outcome(&mut self, forward: bool) -> StepOutcome {
        let state = &mut self.state;
        let total_tabs = state.diff_tabs.len();
        if total_tabs == 0 {
            return state
                .diff
                .as_ref()
                .and_then(|diff| diff.diff_state.clone())
                .map_or(StepOutcome::None, StepOutcome::WrapSingleState);
        }

        let active_idx = state.active_diff_tab.unwrap_or(0).min(total_tabs - 1);
        let hunks_len = state.diff_tabs[active_idx].hunks.len();
        let current_hunk = state.diff_tabs[active_idx].current_hunk;

        if hunks_len == 0 {
            if total_tabs > 1 {
                let next_idx = if forward {
                    (active_idx + 1) % total_tabs
                } else {
                    (active_idx + total_tabs - 1) % total_tabs
                };
                if let Some(target) = state.diff_tabs.get_mut(next_idx) {
                    target.pending_edge_jump = Some(forward);
                }
                return StepOutcome::SwitchTab(next_idx);
            }
            return state.diff_tabs[active_idx]
                .diff_state
                .clone()
                .map_or(StepOutcome::None, StepOutcome::WrapSingleState);
        }

        let hunk_idx = match next_hunk_index(current_hunk, hunks_len, forward) {
            Ok(idx) => idx,
            Err(from_start) if total_tabs > 1 => {
                let target_idx = if from_start {
                    (active_idx + 1) % total_tabs
                } else {
                    (active_idx + total_tabs - 1) % total_tabs
                };
                if let Some(target) = state.diff_tabs.get_mut(target_idx) {
                    target.pending_edge_jump = Some(from_start);
                }
                return StepOutcome::SwitchTab(target_idx);
            }
            Err(from_start) => {
                if from_start {
                    0
                } else {
                    hunks_len - 1
                }
            }
        };

        let active = &mut state.diff_tabs[active_idx];
        active.current_hunk = Some(hunk_idx);
        let pos = active.hunks[hunk_idx].gutter_position();
        let ds = active.diff_state.clone();
        state.diff = Some(active.clone());
        ds.map_or(StepOutcome::None, |ds| StepOutcome::JumpInCurrent(ds, pos))
    }

    /// 手动刷新当前激活的差异视图。
    pub(crate) fn refresh_active_diff(&mut self, cx: &mut Context<Self>) {
        self.reload_active_diff(false, cx);
    }

    /// 当状态快照更新时，后台静默重拉涉及工作区或暂存区的激活差异，避免内容过期。
    pub(crate) fn refresh_active_worktree_diff(&mut self, cx: &mut Context<Self>) {
        self.reload_active_diff(true, cx);
    }

    fn reload_active_diff(&mut self, mutable_only: bool, cx: &mut Context<Self>) {
        let Some(active) = self.state.active_diff() else {
            return;
        };
        if mutable_only && active.loading {
            return;
        }
        let Some(req) = active.request.clone() else {
            return;
        };
        if mutable_only
            && !matches!(req.old, GitSide::Worktree | GitSide::Index)
            && !matches!(req.new, GitSide::Worktree | GitSide::Index)
        {
            return;
        }
        let old_id = active.request_id;
        let new_id = GitRequestId::new();
        self.state.pending.push(new_id);
        self.diff_tasks.remove(&old_id);

        let refresh_slot = |slot: &mut GitDiffState| {
            slot.request_id = new_id;
            slot.error = None;
            if !mutable_only {
                slot.loading = true;
            }
        };
        if let Some(d) = self.state.diff.as_mut()
            && d.request_id == old_id
        {
            refresh_slot(d);
        }
        if let Some(tab) = self
            .state
            .diff_tabs
            .iter_mut()
            .find(|t| t.request_id == old_id)
        {
            refresh_slot(tab);
        }

        let project = self.state.project.clone();
        self.send(GitCommand::LoadDiff {
            project,
            request_id: new_id,
            path: req.path,
            old: req.old,
            new: req.new,
        });
        cx.notify();
    }

    /// 用系统默认程序打开当前差异对应的仓库源文件（带路径防穿越校验）。
    pub(crate) fn open_diff_source(&self) {
        let Some(active) = self.state.active_diff() else {
            return;
        };
        let rel = Path::new(&active.path);
        if rel.is_absolute() {
            return;
        }
        let Ok(canonical_root) = self.state.project.path.canonicalize() else {
            return;
        };
        let Ok(canonical_target) = canonical_root.join(rel).canonicalize() else {
            return;
        };
        if !canonical_target.starts_with(&canonical_root) {
            log::warn!(
                "拒绝打开越界差异源文件，路径: {}，仓库根: {}",
                active.path,
                canonical_root.display()
            );
            return;
        }
        open_local_path(&canonical_target);
    }

    /// 批量关闭差异标签（关闭其他 / 左侧 / 右侧 / 全部），并保持或合理迁移激活项。
    pub(crate) fn close_diff_tabs_scope(
        &mut self,
        anchor_request_id: Option<GitRequestId>,
        scope: DiffTabCloseScope,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.state;
        if state.diff_tabs.is_empty() {
            return;
        }
        let anchor_idx = anchor_request_id
            .and_then(|req| state.diff_tabs.iter().position(|tab| tab.request_id == req));
        if scope != DiffTabCloseScope::All && anchor_idx.is_none() {
            return;
        }
        let anchor = anchor_idx.unwrap_or(0);
        let active_req = state
            .active_diff_tab
            .and_then(|idx| state.diff_tabs.get(idx))
            .map(|tab| tab.request_id);

        let mut kept = Vec::with_capacity(state.diff_tabs.len());
        let mut removed_reqs = Vec::new();
        for (idx, tab) in state.diff_tabs.drain(..).enumerate() {
            let keep = match scope {
                DiffTabCloseScope::All => false,
                DiffTabCloseScope::Other => idx == anchor,
                DiffTabCloseScope::Left => idx >= anchor,
                DiffTabCloseScope::Right => idx <= anchor,
            };
            if keep {
                kept.push(tab);
            } else {
                removed_reqs.push(tab.request_id);
            }
        }
        if removed_reqs.is_empty() {
            state.diff_tabs = kept;
            return;
        }
        for req in removed_reqs {
            self.diff_tasks.remove(&req);
        }
        state.diff_tabs = kept;
        if state.diff_tabs.is_empty() {
            state.active_diff_tab = None;
            state.diff = None;
            recompute_workspace_diff_search(state);
            cx.notify();
            return;
        }

        let next_idx = active_req
            .and_then(|req| state.diff_tabs.iter().position(|t| t.request_id == req))
            .unwrap_or(0);
        state.active_diff_tab = Some(next_idx);
        state.diff = state.diff_tabs.get(next_idx).cloned();
        let maybe_req = state
            .diff_tabs
            .get(next_idx)
            .and_then(|tab| diff_tab_needs_recompute(tab, state.diff_options.ignore_whitespace));
        recompute_workspace_diff_search(state);
        if let Some(request_id) = maybe_req {
            self.schedule_diff_prepare(request_id, cx);
        }
        cx.notify();
    }
}
