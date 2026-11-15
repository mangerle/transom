//! Git 日志引用面板、跳转与分页加载动作。

use super::*;
use crate::git::types::{LogColumn, LogJump};

impl GitWorkspace {
    /// 折叠 / 展开引用栏的一个分组。
    pub(crate) fn toggle_refs_group(
        &mut self,
        key: &str,
        default_expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let expanded = &mut self.state.refs_expanded;
        let current = expanded.get(key).copied().unwrap_or(default_expanded);
        expanded.insert(key.to_string(), !current);
        cx.notify();
    }

    /// 一次性展开或收起引用栏的四个引用分组。
    pub(crate) fn set_all_ref_groups_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        for key in ["HEAD", "git.refs.local", "git.refs.remote", "git.refs.tags"] {
            self.state.refs_expanded.insert(key.to_string(), expanded);
        }
        cx.notify();
    }

    /// 切换日志表的一列显示（表头右键菜单）。
    pub(crate) fn toggle_log_column(&mut self, column: LogColumn, cx: &mut Context<Self>) {
        column.flip(&mut self.state.log.columns);
        cx.notify();
    }

    /// 转到父提交 / 子提交（IDEA 提交右键菜单的两条跳转）。
    pub(crate) fn jump_commit(&mut self, jump: LogJump, cx: &mut Context<Self>) {
        if let Some(index) = jump.resolve(&self.state.log.entries) {
            self.select_commit_by_index(index, cx);
            return;
        }
        self.state.log.jump = Some(jump);
        if self.state.log.has_more && !self.state.log.loading {
            self.load_more_log(cx);
            return;
        }
        cx.emit(GitWorkspaceEvent::TransientStatus(
            rust_i18n::t!("git.log.jump_missing").to_string(),
        ));
        cx.notify();
    }

    /// 按提交 OID 选择提交；刷新或插入新提交后仍指向同一条记录。
    pub(crate) fn select_commit_by_oid(&mut self, oid: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.state.log.entries.iter().position(|e| e.oid == oid) {
            self.select_commit_by_index(index, cx);
        }
    }

    /// 选中一条提交（渲染层按行号回调）。
    pub(crate) fn select_commit_by_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(oid) = self.state.log.entries.get(index).map(|e| e.oid.clone()) else {
            return;
        };
        self.state.selected_commit = Some(index);
        self.state.collapsed_log_dirs.clear();
        self.state.commit_files = Vec::new();
        self.state.commit_files_loading = true;
        self.state.diff = None;
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        let project = self.state.project.clone();
        self.send(GitCommand::LoadCommitFiles {
            project,
            request_id,
            oid,
        });
        cx.notify();
    }

    /// 加载更多历史（追加页）。
    pub(crate) fn load_more_log(&mut self, cx: &mut Context<Self>) {
        let len = self.state.log.entries.len();
        self.load_log_notified(len, cx);
    }

    /// 提交清单里点一个文件：与该提交的第一父比较。
    pub(crate) fn diff_commit_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(oid) = self.state.selected_entry().map(|entry| entry.oid.clone()) else {
            return;
        };
        self.open_diff(
            GitDiffRequest {
                path: path.to_string(),
                old: GitSide::Commit(format!("{oid}^1")),
                new: GitSide::Commit(oid),
                left_label: rust_i18n::t!("git.diff.parent").to_string(),
                right_label: rust_i18n::t!("git.diff.commit").to_string(),
            },
            cx,
        );
    }
}
