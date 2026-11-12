//! Git 差异跨文件块导航、主动/自动刷新与源码跳转打开动作。
//!
//! 设计原理（Why）：
//! - 将跨文件差异步进、`StatusLoaded` 自动重载及工具栏刷新/打开源文件等逻辑从
//!   `diff_actions.rs` 拆分，避免单文件膨胀超过 500 行阈值；
//! - 跨文件导航以标签页内 `current_hunk` 游标与目标页 `pending_edge_jump` 为状态载体，
//!   既能平滑跨越尚未计算完的标签边界，又不污染渲染层状态。

use std::path::Path;

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId, GitSide, TabId};

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

impl AppShell {
    /// 上一处/下一处差异：到达当前文件首尾边界时自动切入上/下一差异标签。
    pub(crate) fn git_step_diff_change(
        &mut self,
        tab_id: TabId,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        enum StepOutcome {
            None,
            WrapSingleState(Entity<gpui_kit::component::diff::DiffState>),
            JumpInCurrent(
                Entity<gpui_kit::component::diff::DiffState>,
                gpui_kit::component::diff::DiffLinePosition,
            ),
            SwitchTab(usize),
        }

        let outcome = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            let total_tabs = state.diff_tabs.len();
            if total_tabs == 0 {
                let Some(diff) = state.diff.as_ref() else {
                    return StepOutcome::None;
                };
                return match diff.diff_state.clone() {
                    Some(ds) => StepOutcome::WrapSingleState(ds),
                    None => StepOutcome::None,
                };
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
                return match state.diff_tabs[active_idx].diff_state.clone() {
                    Some(ds) => StepOutcome::WrapSingleState(ds),
                    None => StepOutcome::None,
                };
            }

            let next_hunk = if forward {
                match current_hunk {
                    None => Some(0),
                    Some(cur) if cur + 1 < hunks_len => Some(cur + 1),
                    Some(_) if total_tabs > 1 => {
                        let next_idx = (active_idx + 1) % total_tabs;
                        if let Some(target) = state.diff_tabs.get_mut(next_idx) {
                            target.pending_edge_jump = Some(true);
                        }
                        return StepOutcome::SwitchTab(next_idx);
                    }
                    Some(_) => Some(0),
                }
            } else {
                match current_hunk {
                    None => Some(hunks_len - 1),
                    Some(cur) if cur > 0 => Some(cur - 1),
                    Some(_) if total_tabs > 1 => {
                        let prev_idx = (active_idx + total_tabs - 1) % total_tabs;
                        if let Some(target) = state.diff_tabs.get_mut(prev_idx) {
                            target.pending_edge_jump = Some(false);
                        }
                        return StepOutcome::SwitchTab(prev_idx);
                    }
                    Some(_) => Some(hunks_len - 1),
                }
            };

            let Some(hunk_idx) = next_hunk else {
                return StepOutcome::None;
            };
            let active = &mut state.diff_tabs[active_idx];
            active.current_hunk = Some(hunk_idx);
            let pos = active.hunks[hunk_idx].gutter_position();
            let ds = active.diff_state.clone();
            state.diff = Some(active.clone());
            match ds {
                Some(ds) => StepOutcome::JumpInCurrent(ds, pos),
                None => StepOutcome::None,
            }
        });

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
                self.git_select_diff_tab(tab_id, target_idx, cx);
            }
        }
    }

    /// 手动刷新当前激活的差异视图。
    pub(crate) fn git_refresh_active_diff(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.git_reload_active_diff(tab_id, false, cx);
    }

    /// 当状态快照更新时，后台静默重拉涉及工作区或暂存区的激活差异，避免内容过期。
    pub(crate) fn git_refresh_active_worktree_diff(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        self.git_reload_active_diff(tab_id, true, cx);
    }

    fn git_reload_active_diff(
        &mut self,
        tab_id: TabId,
        mutable_only: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let job = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            let active = state.active_diff()?;
            if mutable_only && active.loading {
                return None;
            }
            let req = active.request.clone()?;
            if mutable_only
                && !matches!(req.old, GitSide::Worktree | GitSide::Index)
                && !matches!(req.new, GitSide::Worktree | GitSide::Index)
            {
                return None;
            }
            let old_id = active.request_id;
            let new_id = GitRequestId::new();
            state.pending.push(new_id);
            workspace.diff_tasks.remove(&old_id);

            let refresh_slot = |slot: &mut crate::git::types::GitDiffState| {
                slot.request_id = new_id;
                slot.error = None;
                if !mutable_only {
                    slot.loading = true;
                }
            };
            if let Some(d) = state.diff.as_mut()
                && d.request_id == old_id
            {
                refresh_slot(d);
            }
            if let Some(tab) = state.diff_tabs.iter_mut().find(|t| t.request_id == old_id) {
                refresh_slot(tab);
            }
            Some((state.project.clone(), new_id, req))
        });

        let Some((project, request_id, req)) = job else {
            return;
        };
        self.send_command(GitCommand::LoadDiff {
            project,
            request_id,
            path: req.path,
            old: req.old,
            new: req.new,
        });
        cx.notify();
    }

    /// 用系统默认程序打开当前差异对应的仓库源文件（带路径防穿越校验）。
    pub(crate) fn git_open_diff_source(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (root, rel_path) = {
            let state = &workspace.read(cx).state;
            let Some(active) = state.active_diff() else {
                return;
            };
            (state.project.path.clone(), active.path.clone())
        };
        let rel = Path::new(&rel_path);
        if rel.is_absolute() {
            return;
        }
        let Ok(canonical_root) = root.canonicalize() else {
            return;
        };
        let Ok(canonical_target) = canonical_root.join(rel).canonicalize() else {
            return;
        };
        if !canonical_target.starts_with(&canonical_root) {
            log::warn!(
                "拒绝打开越界差异源文件，路径: {}，仓库根: {}",
                rel_path,
                canonical_root.display()
            );
            return;
        }
        crate::audit::open_local_path(&canonical_target);
    }

    /// 批量关闭差异标签（关闭其他 / 左侧 / 右侧 / 全部），并保持或合理迁移激活项。
    pub(crate) fn git_close_diff_tabs_scope(
        &mut self,
        tab_id: TabId,
        anchor_request_id: Option<GitRequestId>,
        scope: DiffTabCloseScope,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let outcome = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            if state.diff_tabs.is_empty() {
                return None;
            }
            let anchor_idx = anchor_request_id
                .and_then(|req| state.diff_tabs.iter().position(|tab| tab.request_id == req));
            if scope != DiffTabCloseScope::All && anchor_idx.is_none() {
                return None;
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
                return None;
            }
            for req in removed_reqs {
                workspace.diff_tasks.remove(&req);
            }
            state.diff_tabs = kept;
            if state.diff_tabs.is_empty() {
                state.active_diff_tab = None;
                state.diff = None;
                super::diff_search::recompute_workspace_diff_search(state);
                return Some((state.project.id, None));
            }
            let next_idx = active_req
                .and_then(|req| state.diff_tabs.iter().position(|t| t.request_id == req))
                .unwrap_or(0);
            state.active_diff_tab = Some(next_idx);
            state.diff = state.diff_tabs.get(next_idx).cloned();
            let maybe_req = state.diff_tabs.get(next_idx).and_then(|tab| {
                super::diff_actions::diff_tab_needs_recompute(
                    tab,
                    state.diff_options.ignore_whitespace,
                )
            });
            super::diff_search::recompute_workspace_diff_search(state);
            Some((state.project.id, maybe_req))
        });
        if let Some((project_id, maybe_req)) = outcome {
            if let Some(request_id) = maybe_req {
                self.schedule_git_diff_prepare(project_id, request_id, cx);
            }
            cx.notify();
        }
    }
}
