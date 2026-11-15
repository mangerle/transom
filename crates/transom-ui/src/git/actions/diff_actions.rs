//! Git 文件差异标签与视图动作。
//!
//! 设计原理（Why）：
//! - 将差异打开、标签选择与标签关闭等操作独立成 `GitWorkspace` 的子模块，
//!   降低单文件复杂度并强化「一个标签一个名字」的多文件对比体验；
//! - 多标签按路径查重：重复打开同一文件时激活对应已开标签，避免产生多个重复标签；
//! - 状态读写与命令投递都直接落在标签页自己的 `GitWorkspace` 实体上，不再经过外壳。

use super::diff_search::recompute_workspace_diff_search;
use crate::app::{app_settings, spawn_blocking_local, update_app_settings};
use crate::diff_pane::{
    IgnoreWhitespace, PreparedDiffDocument, create_diff_state, prepare_diff_document,
    sync_diff_state_options,
};
use crate::editor::editor_language_token;
use crate::git::types::{GitDiffOptions, GitDiffRequest, GitDiffState};
use crate::git::{GitWorkspace, GitWorkspaceEvent};
use gpui_kit::Context;
use transom_core::{DiffContent, GitAction, GitCommand, GitRequestId};

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

impl GitWorkspace {
    /// 打开一个文件的差异视图，并以标签页形式管理。
    pub(crate) fn open_diff(&mut self, request: GitDiffRequest, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        self.state.last_diff = Some(request.clone());

        let new_diff = GitDiffState::loading(request.clone(), request_id);
        self.state.diff = Some(new_diff.clone());

        // 多标签管理：按「文件路径 + 对比两侧」查重，已存在则替换为最新加载状态并激活
        if let Some(pos) = self.state.diff_tabs.iter().position(|t| {
            t.request
                .as_ref()
                .is_some_and(|existing| existing.same_target(&request))
                || (t.request.is_none() && t.path == request.path)
        }) {
            let replaced_req = self.state.diff_tabs[pos].request_id;
            self.state.diff_tabs[pos] = new_diff;
            self.state.active_diff_tab = Some(pos);
            self.diff_tasks.remove(&replaced_req);
        } else {
            self.state.diff_tabs.push(new_diff);
            self.state.active_diff_tab = Some(self.state.diff_tabs.len() - 1);
        }

        self.send(GitCommand::LoadDiff {
            project,
            request_id,
            path: request.path,
            old: request.old,
            new: request.new,
        });
        cx.notify();
    }

    /// 对单个变更块应用补丁（暂存块 / 取消暂存块 / 还原块）。
    pub(crate) fn apply_hunk_patch(
        &mut self,
        action: GitAction,
        patch: String,
        cached: bool,
        reverse: bool,
        cx: &mut Context<Self>,
    ) {
        let project = self.state.project.clone();
        self.send(GitCommand::ApplyHunkPatch {
            project,
            action,
            patch,
            cached,
            reverse,
        });
        cx.notify();
    }

    /// 在后台线程池计算指定请求的 Myers 差异与语法高亮前置补丁，并缓存到 `GitDiffState`。
    pub(crate) fn schedule_diff_prepare(
        &mut self,
        request_id: GitRequestId,
        cx: &mut Context<Self>,
    ) {
        let Some((content, token, ignore_whitespace)) = self.begin_diff_prepare(request_id) else {
            return;
        };
        let language = editor_language_token(&content.path);
        let task = cx.spawn(async move |this, cx| {
            let prepared = spawn_blocking_local(move || {
                prepare_diff_document(
                    &content.old_text,
                    &content.new_text,
                    &content.path,
                    &language,
                    ignore_whitespace,
                )
            })
            .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.finish_diff_prepare(request_id, token, ignore_whitespace, prepared, cx);
            });
        });
        self.diff_tasks.insert(request_id, task);
    }

    fn begin_diff_prepare(
        &mut self,
        request_id: GitRequestId,
    ) -> Option<(DiffContent, u64, IgnoreWhitespace)> {
        let ignore_whitespace = self.state.diff_options.ignore_whitespace;
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

        if let Some(current) = self.state.diff.as_mut() {
            update_slot(current);
        }
        for tab in &mut self.state.diff_tabs {
            update_slot(tab);
        }

        target_content.map(|content| (content, next_token, ignore_whitespace))
    }

    fn finish_diff_prepare(
        &mut self,
        request_id: GitRequestId,
        token: u64,
        ignore_whitespace: IgnoreWhitespace,
        prepared: Option<PreparedDiffDocument>,
        cx: &mut Context<Self>,
    ) {
        if !self.state.diff_slots_match(request_id, token) {
            return;
        }
        self.diff_tasks.remove(&request_id);
        let Some(prepared) = prepared else {
            self.state.update_diff_slots(request_id, token, |slot| {
                slot.computing = false;
                slot.error = Some(rust_i18n::t!("git.diff.compute_failed").to_string());
            });
            cx.notify();
            return;
        };

        let options = self.state.diff_options;
        if options.ignore_whitespace != ignore_whitespace {
            // 后台计算期间切换了标签并修改全局空白选项：必须清除 computing，
            // 避免下次切回该标签时因 `!computing` 为假而永久卡死。
            self.state.update_diff_slots(request_id, token, |slot| {
                slot.computing = false;
            });
            cx.notify();
            return;
        }

        let diff_entity =
            (!prepared.is_empty()).then(|| create_diff_state(prepared.files, options, cx));
        let additions = prepared.additions;
        let deletions = prepared.deletions;
        let hunks = prepared.hunk_count;
        let prepared_hunks = prepared.hunks;
        let mut edge_jump: Option<usize> = None;
        let ok = self.state.update_diff_slots(request_id, token, |slot| {
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
            && self
                .state
                .active_diff()
                .is_some_and(|active| active.request_id == request_id)
        {
            recompute_workspace_diff_search(&mut self.state);
        }
        if let (Some(ds), Some(hunk_idx)) = (diff_entity.as_ref(), edge_jump)
            && let Some(hunk) = prepared_hunks.get(hunk_idx)
        {
            let pos = hunk.gutter_position();
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
        }
        if ok {
            cx.notify();
        }
    }

    /// 修改一个差异显示选项并同步/重算当前差异。
    pub(crate) fn update_diff_options(
        &mut self,
        update: impl FnOnce(&mut GitDiffOptions),
        cx: &mut Context<Self>,
    ) {
        let old_ignore = self.state.diff_options.ignore_whitespace;
        update(&mut self.state.diff_options);
        self.apply_diff_options_change(old_ignore, cx);
    }

    /// 同步 Git 差异显示选项变化：空白忽略变化走后台重算，其余外观选项直接原地同步 `DiffState`。
    pub(crate) fn apply_diff_options_change(
        &mut self,
        old_ignore: IgnoreWhitespace,
        cx: &mut Context<Self>,
    ) {
        let new_options = self.state.diff_options;
        let active = self.state.active_diff();
        let active_req = active
            .filter(|d| !d.loading && d.content.as_ref().is_some_and(|c| !c.binary))
            .map(|d| d.request_id);
        let active_state = active.and_then(|d| d.diff_state.clone());

        if new_options.ignore_whitespace != old_ignore {
            if let Some(request_id) = active_req {
                self.schedule_diff_prepare(request_id, cx);
            }
        } else if let Some(ds) = active_state {
            // 仅立即同步当前激活标签，后台标签在切回时懒同步，避免多文件大变更时冗余重排
            sync_diff_state_options(&ds, new_options, cx);
        }
        update_app_settings(cx, |s| new_options.write_to_git_settings(&mut s.git));
        let snapshot = app_settings(cx).clone();
        cx.emit(GitWorkspaceEvent::PersistSettings(Box::new(snapshot)));
        cx.notify();
    }

    /// 切换当前激活的差异文件标签。
    pub(crate) fn select_diff_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.state.diff_tabs.len() {
            return;
        }
        let options = self.state.diff_options;
        self.state.active_diff_tab = Some(index);
        let Some(active) = self.state.diff_tabs.get_mut(index) else {
            return;
        };
        let maybe_req = diff_tab_needs_recompute(active, options.ignore_whitespace);
        let cached_ds = maybe_req
            .is_none()
            .then(|| active.diff_state.clone())
            .flatten();
        let edge_pos = if cached_ds.is_some()
            && let Some(from_start) = active.pending_edge_jump.take()
            && !active.hunks.is_empty()
        {
            let hunk_idx = if from_start {
                0
            } else {
                active.hunks.len() - 1
            };
            active.current_hunk = Some(hunk_idx);
            active.hunks.get(hunk_idx).map(|h| h.gutter_position())
        } else {
            None
        };
        self.state.diff = Some(active.clone());
        recompute_workspace_diff_search(&mut self.state);

        if let Some(ds) = cached_ds {
            sync_diff_state_options(&ds, options, cx);
            if let Some(pos) = edge_pos {
                ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
            }
        }
        if let Some(request_id) = maybe_req {
            self.schedule_diff_prepare(request_id, cx);
        }
        cx.notify();
    }

    /// 按请求标识切换差异标签，避免同路径多对比视角时索引混淆。
    pub(crate) fn select_diff_tab_by_request(
        &mut self,
        request_id: GitRequestId,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .state
            .diff_tabs
            .iter()
            .position(|tab| tab.request_id == request_id)
        {
            self.select_diff_tab(index, cx);
        }
    }

    /// 按请求标识关闭单个差异标签。
    pub(crate) fn close_diff_tab_by_request(
        &mut self,
        request_id: GitRequestId,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .state
            .diff_tabs
            .iter()
            .position(|tab| tab.request_id == request_id)
        {
            self.close_diff_tab(index, cx);
        }
    }

    /// 关闭指定的差异文件标签。
    pub(crate) fn close_diff_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.state.diff_tabs.len() {
            return;
        }
        let removed_tab = self.state.diff_tabs.remove(index);
        self.diff_tasks.remove(&removed_tab.request_id);
        let next_req = if self.state.diff_tabs.is_empty() {
            self.state.active_diff_tab = None;
            self.state.diff = None;
            None
        } else {
            let next_idx = index.min(self.state.diff_tabs.len() - 1);
            self.state.active_diff_tab = Some(next_idx);
            self.state.diff = self.state.diff_tabs.get(next_idx).cloned();
            self.state.diff_tabs.get(next_idx).and_then(|tab| {
                diff_tab_needs_recompute(tab, self.state.diff_options.ignore_whitespace)
            })
        };
        recompute_workspace_diff_search(&mut self.state);
        if let Some(request_id) = next_req {
            self.schedule_diff_prepare(request_id, cx);
        }
        cx.notify();
    }
}
