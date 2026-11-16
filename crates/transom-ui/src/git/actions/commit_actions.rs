//! Git 提交页、改动树、搁置（Shelf）与贮藏（Stash）的工作区实体动作。

use super::GitDiffRequest;
use crate::git::GitCommitTab;
use crate::git::commit_modal::{ChangeGroup, worktree_diff_request};
use crate::git::entity::{GitWorkspace, GitWorkspaceEvent};
use gpui_kit::{Context, Window};
use transom_core::{
    GitAction, GitCommand, GitRequestId, GitSide, HeadState, ShelfEntry, StashFile, StatusRecord,
};

impl GitWorkspace {
    /// 切换单条改动路径的勾选状态。
    pub(crate) fn toggle_checked(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.state.checked.remove(path) {
            self.state.checked.insert(path.to_string());
        }
        cx.notify();
    }

    /// 丢弃当前已勾选路径的改动，已跟踪走 Discard，未跟踪走 DeleteUntracked。
    pub(crate) fn rollback_checked(&mut self, cx: &mut Context<Self>) {
        if self.state.checked.is_empty() {
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("git.commit.nothing_selected").to_string(),
            ));
            return;
        }
        let mut tracked = Vec::new();
        let mut untracked = Vec::new();
        if let Some(status) = self.state.status.as_ref() {
            for entry in &status.entries {
                if !self.state.checked.contains(&entry.path) {
                    continue;
                }
                if matches!(entry.record, StatusRecord::Untracked) {
                    untracked.push(entry.path.clone());
                } else {
                    tracked.push(entry.path.clone());
                }
            }
        }
        self.state.checked.clear();
        if !tracked.is_empty() {
            self.run_paths(GitAction::Discard, tracked, cx);
        }
        if !untracked.is_empty() {
            self.run_paths(GitAction::DeleteUntracked, untracked, cx);
        }
    }

    /// 在右侧差异区打开工作区单个文件的差异。
    pub(crate) fn open_worktree_diff(
        &mut self,
        group: ChangeGroup,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.open_diff(worktree_diff_request(group, path), cx);
    }

    /// 打开当前改动列表里第一个文件的差异。
    pub(crate) fn open_first_worktree_diff(&mut self, cx: &mut Context<Self>) {
        let Some((group, path)) = self.state.status
            .as_ref()
            .and_then(super::first_worktree_diff)
        else {
            return;
        };
        self.open_worktree_diff(group, path, cx);
    }

    /// 选择修正目标提交：更新 amend_target，若已开启修正则回填消息并加载文件。
    pub(crate) fn select_amend_target(
        &mut self,
        oid: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.amend_target = Some(oid.clone());
        if !self.state.commit_amend {
            cx.notify();
            return;
        }
        let Some(entry) = self
            .state
            .log
            .entries
            .iter()
            .find(|entry| entry.oid == oid)
            .or_else(|| self.state.log.entries.first())
            .cloned()
        else {
            cx.notify();
            return;
        };
        let mut message = entry.subject.clone();
        let body = entry.body.trim();
        if !body.is_empty() {
            message.push_str("\n\n");
            message.push_str(body);
        }
        let request_id = GitRequestId::new();
        self.state.amend_request = Some(request_id);
        self.state.amend_files_loading = true;
        self.state.amend_files.clear();
        self.state.pending.push(request_id);

        let commit_message = self.state.commit_message.clone();
        commit_message.update(cx, |input, cx| input.set_value(message, window, cx));
        let project = self.state.project.clone();
        self.send(GitCommand::LoadCommitFiles {
            project,
            request_id,
            oid: entry.oid,
        });
        cx.notify();
    }

    /// 在提交页左栏切换「提交 / 搁置 / 贮藏」；首次进入拉取对应列表。
    pub(crate) fn set_commit_tab(&mut self, tab: GitCommitTab, cx: &mut Context<Self>) {
        if self.state.commit_tab == tab {
            return;
        }
        self.state.commit_tab = tab;
        let project = self.state.project.clone();
        match tab {
            GitCommitTab::Shelf => {
                let request_id = GitRequestId::new();
                self.state.pending.push(request_id);
                self.send(GitCommand::LoadShelves {
                    project,
                    request_id,
                });
            }
            GitCommitTab::Stash => {
                let request_id = GitRequestId::new();
                self.state.pending.push(request_id);
                self.send(GitCommand::ListStashes {
                    project,
                    request_id,
                });
            }
            GitCommitTab::Commit => {}
        }
        cx.notify();
    }

    /// 展开或折叠一个搁置项。
    pub(crate) fn toggle_shelf_expanded(&mut self, entry_id: &str, cx: &mut Context<Self>) {
        self.state.expanded_shelf = if self.state.expanded_shelf.as_deref() == Some(entry_id) {
            None
        } else {
            Some(entry_id.to_string())
        };
        cx.notify();
    }

    /// 重新读取搁置项列表。
    pub(crate) fn reload_shelves(&mut self, cx: &mut Context<Self>) {
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        let project = self.state.project.clone();
        self.send(GitCommand::LoadShelves {
            project,
            request_id,
        });
        cx.notify();
    }

    /// 把工作区勾选的改动创建为独立离线搁置项（Shelf）。
    pub(crate) fn shelve_checked(&mut self, cx: &mut Context<Self>) {
        let files: Vec<String> = if self.state.checked.is_empty() {
            self.state.status
                .as_ref()
                .map(|status| status.entries.iter().map(|e| e.path.clone()).collect())
                .unwrap_or_default()
        } else {
            self.state.checked.iter().cloned().collect()
        };
        if files.is_empty() {
            return;
        }
        let branch = match self.state.status.as_ref().map(|status| &status.head) {
            Some(HeadState::OnBranch(branch)) => branch.clone(),
            Some(HeadState::Initial { branch }) => branch.clone(),
            _ => "HEAD".to_string(),
        };
        let name = format!("{} ({})", rust_i18n::t!("git.shelf.title"), files.len());
        let project = self.state.project.clone();
        self.send(GitCommand::Shelve {
            project,
            request_id: GitRequestId::new(),
            name,
            branch,
            files,
        });
        cx.notify();
    }

    /// 应用或弹回搁置项。
    pub(crate) fn apply_shelf(&mut self, entry_id: &str, pop: bool, cx: &mut Context<Self>) {
        let Some(entry) = self.state.shelves.iter().find(|shelf| shelf.id == entry_id) else {
            return;
        };
        let project = self.state.project.clone();
        let patch = entry.patch.clone();
        self.send(GitCommand::ApplyShelf {
            project,
            request_id: GitRequestId::new(),
            shelf_id: entry_id.to_string(),
            patch,
            pop,
        });
        cx.notify();
    }

    /// 删除指定搁置项。
    pub(crate) fn drop_shelf(&mut self, entry_id: &str, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        self.send(GitCommand::DropShelf {
            project,
            request_id: GitRequestId::new(),
            shelf_id: entry_id.to_string(),
        });
        cx.notify();
    }

    /// 打开搁置项中某文件的差异。
    pub(crate) fn open_shelf_file_diff(
        &mut self,
        entry: &ShelfEntry,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        self.open_diff(
            GitDiffRequest {
                path: path.to_string(),
                old: GitSide::Commit("HEAD".to_string()),
                new: GitSide::Worktree,
                left_label: format!("{} ({})", rust_i18n::t!("git.diff.baseline"), entry.branch),
                right_label: rust_i18n::t!("git.shelf.title").to_string(),
            },
            cx,
        );
    }

    /// 把工作区勾选的改动保存到 Git Stash 栈。
    pub(crate) fn stash_checked(&mut self, cx: &mut Context<Self>) {
        let message = if self.state.checked.is_empty() {
            rust_i18n::t!("git.stash.auto_message").to_string()
        } else {
            format!(
                "{} ({})",
                rust_i18n::t!("git.stash.title"),
                self.state.checked.len()
            )
        };
        let project = self.state.project.clone();
        self.state.checked.clear();
        self.send(GitCommand::StashSave {
            project,
            message,
            include_untracked: false,
        });
        cx.notify();
    }

    /// 重新拉取贮藏列表。
    pub(crate) fn reload_stashes(&mut self, cx: &mut Context<Self>) {
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        let project = self.state.project.clone();
        self.send(GitCommand::ListStashes {
            project,
            request_id,
        });
        cx.notify();
    }

    /// 对贮藏项执行 apply / pop / drop 动作。
    pub(crate) fn run_stash_action(
        &mut self,
        selector: String,
        action: GitAction,
        cx: &mut Context<Self>,
    ) {
        let project = self.state.project.clone();
        let command = match action {
            GitAction::StashApply => GitCommand::StashApply { project, selector },
            GitAction::StashPop => GitCommand::StashPop { project, selector },
            GitAction::StashDrop => GitCommand::StashDrop { project, selector },
            other => {
                log::error!("非预期的搁置动作: {other:?}");
                return;
            }
        };
        self.send(command);
        cx.notify();
    }

    /// 展开/折叠贮藏行下的改动文件列表；每次展开重新拉取以避开栈编号前移过期。
    pub(crate) fn toggle_stash_files(&mut self, selector: &str, cx: &mut Context<Self>) {
        let opening = self.state.expanded_stash.as_deref() != Some(selector);
        self.state.expanded_stash = opening.then(|| selector.to_string());
        self.state.stash_files.clear();
        self.state.stash_files_loading = false;
        if !opening {
            cx.notify();
            return;
        }
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        self.state.stash_files_loading = true;
        let project = self.state.project.clone();
        self.send(GitCommand::ListStashFiles {
            project,
            request_id,
            selector: selector.to_string(),
        });
        cx.notify();
    }

    /// 打开某条贮藏里某文件相对父提交的差异。
    pub(crate) fn open_stash_diff(
        &mut self,
        selector: &str,
        file: &StashFile,
        cx: &mut Context<Self>,
    ) {
        let stashed = if file.untracked {
            format!("{selector}^3")
        } else {
            selector.to_string()
        };
        self.open_diff(
            GitDiffRequest {
                path: file.path.clone(),
                old: GitSide::Commit(format!("{selector}^")),
                new: GitSide::Commit(stashed),
                left_label: rust_i18n::t!("git.diff.baseline").to_string(),
                right_label: rust_i18n::t!("git.diff.stashed").to_string(),
            },
            cx,
        );
    }
}
