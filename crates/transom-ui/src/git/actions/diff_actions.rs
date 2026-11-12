//! Git 文件差异标签与视图动作。
//!
//! 设计原理（Why）：
//! - 将差异打开、标签选择与标签关闭等操作从 `log_actions` 独立出来，
//!   降低单文件复杂度并强化「一个标签一个名字」的多文件对比体验；
//! - 多标签按路径查重：重复打开同一文件时激活对应已开标签，避免产生多个重复标签；
//! - 状态读写都落在标签页自己的 `Entity<GitWorkspace>` 上，命令走外壳的命令总线。

use crate::app::{AppShell, app_settings, update_app_settings};
use crate::diff_pane::{
    DiffViewOptions, IgnoreWhitespace, create_diff_state, prepare_diff_document,
    sync_diff_state_options,
};
use crate::git::types::{GitDiffRequest, GitDiffState};
use gpui_kit::*;
use transom_core::{GitCommand, GitProjectId, GitRequestId, TabId};

/// 判断被切换为激活态的差异标签是否需要按当前空白规则重新计算。
pub(super) fn diff_tab_needs_recompute(
    tab: &GitDiffState,
    ignore: IgnoreWhitespace,
) -> Option<GitRequestId> {
    let needs = !tab.loading
        && !tab.computing
        && tab.error.is_none()
        && tab.content.as_ref().is_some_and(|c| !c.binary)
        && tab.computed_ignore != Some(ignore);
    needs.then_some(tab.request_id)
}

impl AppShell {
    /// 打开一个文件的差异视图，并以标签页形式管理。
    pub(crate) fn git_open_diff(
        &mut self,
        tab_id: TabId,
        request: GitDiffRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let project = workspace.read(cx).state.project.clone();
        let request_id = GitRequestId::new();
        workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.pending.push(request_id);
            state.last_diff = Some(request.clone());

            let new_diff = GitDiffState::loading(request.clone(), request_id);
            state.diff = Some(new_diff.clone());

            // 多标签管理：按「文件路径 + 对比两侧」查重，已存在则替换为最新加载状态并激活
            if let Some(pos) = state.diff_tabs.iter().position(|t| {
                t.request
                    .as_ref()
                    .is_some_and(|existing| existing.same_target(&request))
                    || (t.request.is_none() && t.path == request.path)
            }) {
                let replaced_req = state.diff_tabs[pos].request_id;
                state.diff_tabs[pos] = new_diff;
                state.active_diff_tab = Some(pos);
                workspace.diff_tasks.remove(&replaced_req);
            } else {
                state.diff_tabs.push(new_diff);
                state.active_diff_tab = Some(state.diff_tabs.len() - 1);
            }
        });

        self.send_command(GitCommand::LoadDiff {
            project,
            request_id,
            path: request.path,
            old: request.old,
            new: request.new,
        });
        cx.notify();
    }

    /// 对单个变更块应用补丁（暂存块 / 取消暂存块 / 还原块）。
    pub(crate) fn git_apply_hunk_patch(
        &mut self,
        tab_id: TabId,
        action: transom_core::GitAction,
        patch: String,
        cached: bool,
        reverse: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let project = workspace.read(cx).state.project.clone();
        self.send_command(GitCommand::ApplyHunkPatch {
            project,
            action,
            patch,
            cached,
            reverse,
        });
        cx.notify();
    }

    /// 在后台线程池计算指定请求的 Myers 差异与语法高亮前置补丁，并缓存到 `GitDiffState`。
    pub(crate) fn schedule_git_diff_prepare(
        &mut self,
        project_id: GitProjectId,
        request_id: GitRequestId,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) else {
            return;
        };
        let job = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            let ignore_whitespace = state.diff_options.ignore_whitespace;

            let mut target_content = None;
            let mut next_token = 0u64;
            let mut update_slot = |slot: &mut GitDiffState| {
                if slot.request_id != request_id {
                    return;
                }
                let Some(content) = slot.content.as_ref() else {
                    return;
                };
                if content.binary {
                    slot.computing = false;
                    slot.diff_state = None;
                    return;
                }
                if next_token == 0 {
                    next_token = slot.compute_token.wrapping_add(1).max(1);
                }
                slot.compute_token = next_token;
                slot.computing = true;
                slot.error = None;
                if target_content.is_none() {
                    target_content = Some(content.clone());
                }
            };

            if let Some(current) = state.diff.as_mut() {
                update_slot(current);
            }
            for tab in &mut state.diff_tabs {
                update_slot(tab);
            }

            target_content.map(|content| (content, next_token, ignore_whitespace))
        });

        let Some((content, token, ignore_whitespace)) = job else {
            return;
        };
        let language = crate::editor::editor_language_token(&content.path);
        let task = cx.spawn(async move |this, cx| {
            let prepared = crate::app::spawn_blocking_local(move || {
                prepare_diff_document(
                    &content.old_text,
                    &content.new_text,
                    &content.path,
                    &language,
                    ignore_whitespace,
                )
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                let Some(workspace) = this.state.git_workspace_of_project(project_id, cx) else {
                    return;
                };
                enum ApplyPlan {
                    Ignore,
                    ResetStale,
                    Failed,
                    Ready { options: DiffViewOptions },
                }
                let plan = workspace.update(cx, |workspace, _| {
                    if !workspace.state.diff_slots_match(request_id, token) {
                        return ApplyPlan::Ignore;
                    }
                    workspace.diff_tasks.remove(&request_id);
                    if prepared.is_none() {
                        workspace.state.update_diff_slots(request_id, token, |slot| {
                            slot.computing = false;
                            slot.error = Some(rust_i18n::t!("git.diff.compute_failed").to_string());
                        });
                        return ApplyPlan::Failed;
                    }
                    let options = workspace.state.diff_options;
                    if options.ignore_whitespace != ignore_whitespace {
                        // 后台计算期间切换了标签并修改全局空白选项：必须清除 computing，
                        // 避免下次切回该标签时因 `!computing` 为假而永久卡死。
                        workspace.state.update_diff_slots(request_id, token, |slot| {
                            slot.computing = false;
                        });
                        return ApplyPlan::ResetStale;
                    }
                    ApplyPlan::Ready { options }
                });

                match plan {
                    ApplyPlan::Ignore => {}
                    ApplyPlan::ResetStale | ApplyPlan::Failed => {
                        cx.notify();
                    }
                    ApplyPlan::Ready { options } => {
                        let Some(prepared) = prepared else {
                            return;
                        };
                        let diff_entity = (!prepared.is_empty())
                            .then(|| create_diff_state(prepared.files, options, cx));
                        let additions = prepared.additions;
                        let deletions = prepared.deletions;
                        let hunks = prepared.hunk_count;
                        let prepared_hunks = prepared.hunks;
                        let mut edge_jump: Option<usize> = None;
                        let applied = workspace.update(cx, |workspace, _| {
                            let ok = workspace.state.update_diff_slots(request_id, token, |slot| {
                                slot.computing = false;
                                slot.computed_ignore = Some(ignore_whitespace);
                                slot.error = None;
                                slot.summary_additions = additions;
                                slot.summary_deletions = deletions;
                                slot.summary_hunks = hunks;
                                slot.hunks = prepared_hunks.clone();
                                slot.whitespace_only = diff_entity.is_none()
                                    && ignore_whitespace != IgnoreWhitespace::None
                                    && slot
                                        .content
                                        .as_ref()
                                        .is_some_and(|c| !c.binary && c.old_text != c.new_text);
                                slot.diff_state = diff_entity.clone();
                                if let Some(from_start) = slot.pending_edge_jump.take()
                                    && !slot.hunks.is_empty()
                                {
                                    let idx = if from_start { 0 } else { slot.hunks.len() - 1 };
                                    slot.current_hunk = Some(idx);
                                    edge_jump = Some(idx);
                                }
                            });
                            if ok
                                && workspace
                                    .state
                                    .active_diff()
                                    .is_some_and(|active| active.request_id == request_id)
                            {
                                super::diff_search::recompute_workspace_diff_search(
                                    &mut workspace.state,
                                );
                            }
                            ok
                        });
                        if let (Some(ds), Some(hunk_idx)) = (diff_entity.as_ref(), edge_jump)
                            && let Some(hunk) = prepared_hunks.get(hunk_idx)
                        {
                            let pos = hunk.gutter_position();
                            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
                        }
                        if applied {
                            cx.notify();
                        }
                    }
                }
            });
        });
        workspace.update(cx, |workspace, _| {
            workspace.diff_tasks.insert(request_id, task);
        });
    }

    /// 同步 Git 差异显示选项变化：空白忽略变化走后台重算，其余外观选项直接原地同步 `DiffState`。
    pub(crate) fn git_apply_diff_options_change(
        &mut self,
        tab_id: TabId,
        old_ignore: crate::diff_pane::IgnoreWhitespace,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (project_id, new_options, active_req, active_state) = {
            let state = &workspace.read(cx).state;
            let active = state.active_diff();
            let active_req = active
                .filter(|d| !d.loading && d.content.as_ref().is_some_and(|c| !c.binary))
                .map(|d| d.request_id);
            let active_state = active.and_then(|d| d.diff_state.clone());
            (state.project.id, state.diff_options, active_req, active_state)
        };

        if new_options.ignore_whitespace != old_ignore {
            if let Some(request_id) = active_req {
                self.schedule_git_diff_prepare(project_id, request_id, cx);
            }
        } else if let Some(ds) = active_state {
            // 仅立即同步当前激活标签，后台标签在切回时懒同步，避免多文件大变更时冗余重排
            sync_diff_state_options(&ds, new_options, cx);
        }
        update_app_settings(cx, |s| new_options.write_to_git_settings(&mut s.git));
        let snapshot = app_settings(cx).clone();
        self.state.persist_settings_reported(&snapshot);
        cx.notify();
    }

    /// 切换当前激活的差异文件标签。
    pub(crate) fn git_select_diff_tab(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let result = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            if index >= state.diff_tabs.len() {
                return None;
            }
            let options = state.diff_options;
            state.active_diff_tab = Some(index);
            let active = state.diff_tabs.get_mut(index)?;
            let maybe_req = diff_tab_needs_recompute(active, options.ignore_whitespace);
            let cached_ds = maybe_req.is_none().then(|| active.diff_state.clone()).flatten();
            let edge_pos = if cached_ds.is_some()
                && let Some(from_start) = active.pending_edge_jump.take()
                && !active.hunks.is_empty()
            {
                let hunk_idx = if from_start { 0 } else { active.hunks.len() - 1 };
                active.current_hunk = Some(hunk_idx);
                active.hunks.get(hunk_idx).map(|h| h.gutter_position())
            } else {
                None
            };
            state.diff = Some(active.clone());
            super::diff_search::recompute_workspace_diff_search(state);
            Some((state.project.id, options, maybe_req, cached_ds, edge_pos))
        });
        if let Some((project_id, options, maybe_req, cached_ds, edge_pos)) = result {
            if let Some(ds) = cached_ds {
                sync_diff_state_options(&ds, options, cx);
                if let Some(pos) = edge_pos {
                    ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
                }
            }
            if let Some(request_id) = maybe_req {
                self.schedule_git_diff_prepare(project_id, request_id, cx);
            }
            cx.notify();
        }
    }

    /// 按请求标识切换差异标签，避免同路径多对比视角时索引混淆。
    pub(crate) fn git_select_diff_tab_by_request(
        &mut self,
        tab_id: TabId,
        request_id: GitRequestId,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let index = workspace
            .read(cx)
            .state
            .diff_tabs
            .iter()
            .position(|tab| tab.request_id == request_id);
        if let Some(index) = index {
            self.git_select_diff_tab(tab_id, index, cx);
        }
    }

    /// 按请求标识关闭单个差异标签。
    pub(crate) fn git_close_diff_tab_by_request(
        &mut self,
        tab_id: TabId,
        request_id: GitRequestId,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let index = workspace
            .read(cx)
            .state
            .diff_tabs
            .iter()
            .position(|tab| tab.request_id == request_id);
        if let Some(index) = index {
            self.git_close_diff_tab(tab_id, index, cx);
        }
    }

    /// 关闭指定的差异文件标签。
    pub(crate) fn git_close_diff_tab(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let outcome = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            if index >= state.diff_tabs.len() {
                return None;
            }
            let removed_tab = state.diff_tabs.remove(index);
            workspace.diff_tasks.remove(&removed_tab.request_id);
            let next_req = if state.diff_tabs.is_empty() {
                state.active_diff_tab = None;
                state.diff = None;
                None
            } else {
                let next_idx = index.min(state.diff_tabs.len() - 1);
                state.active_diff_tab = Some(next_idx);
                state.diff = state.diff_tabs.get(next_idx).cloned();
                state.diff_tabs.get(next_idx).and_then(|tab| {
                    diff_tab_needs_recompute(tab, state.diff_options.ignore_whitespace)
                })
            };
            super::diff_search::recompute_workspace_diff_search(state);
            Some((state.project.id, next_req))
        });
        if let Some((project_id, maybe_req)) = outcome {
            if let Some(request_id) = maybe_req {
                self.schedule_git_diff_prepare(project_id, request_id, cx);
            }
            cx.notify();
        }
    }
}
