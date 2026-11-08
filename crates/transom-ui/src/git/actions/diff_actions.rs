//! Git 文件差异标签与视图动作。
//!
//! 设计原理（Why）：
//! - 将差异打开、标签选择与标签关闭等操作从 `log_actions` 独立出来，
//!   降低单文件复杂度并强化「一个标签一个名字」的多文件对比体验；
//! - 多标签按路径查重：重复打开同一文件时激活对应已开标签，避免产生多个重复标签；
//! - 状态读写都落在标签页自己的 `Entity<GitWorkspace>` 上，命令走外壳的命令总线。

use crate::app::AppShell;
use crate::git::types::{GitDiffRequest, GitDiffState};
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId, TabId};

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

            let new_diff = GitDiffState {
                path: request.path.clone(),
                left_label: request.left_label.clone(),
                right_label: request.right_label.clone(),
                content: None,
                loading: true,
                error: None,
                request_id,
            };
            state.diff = Some(new_diff.clone());

            // 多标签管理：若已存在相同路径的标签则更新之，否则新增
            if let Some(pos) = state.diff_tabs.iter().position(|t| t.path == request.path) {
                state.diff_tabs[pos] = new_diff;
                state.active_diff_tab = Some(pos);
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
        let selected = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            if index >= state.diff_tabs.len() {
                return false;
            }
            state.active_diff_tab = Some(index);
            state.diff = state.diff_tabs.get(index).cloned();
            true
        });
        if selected {
            cx.notify();
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
        let removed = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            if index >= state.diff_tabs.len() {
                return false;
            }
            state.diff_tabs.remove(index);
            if state.diff_tabs.is_empty() {
                state.active_diff_tab = None;
                state.diff = None;
            } else {
                let next_idx = index.min(state.diff_tabs.len() - 1);
                state.active_diff_tab = Some(next_idx);
                state.diff = state.diff_tabs.get(next_idx).cloned();
            }
            true
        });
        if removed {
            cx.notify();
        }
    }
}
