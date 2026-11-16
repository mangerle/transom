//! Git 工作区实体的引用、远端、工作树与历史改写动作。

use super::GitHistoryAction;
use crate::git::GitNamePurpose;
use crate::git::entity::{GitWorkspace, GitWorkspaceEvent};
use gpui_kit::Context;
use transom_core::{GitCheckout, GitCommand, GitFoldMode, HeadState};

impl GitWorkspace {
    /// 触发远端抓取（Fetch）。
    pub(crate) fn fetch(&mut self, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        if self.send(GitCommand::Fetch { project }) {
            cx.emit(GitWorkspaceEvent::TransientStatus(
                rust_i18n::t!("git.action.fetching").to_string(),
            ));
        }
        cx.notify();
    }

    /// 触发远端拉取（Pull）。
    pub(crate) fn pull(&mut self, rebase: bool, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        if self.send(GitCommand::Pull { project, rebase }) {
            cx.emit(GitWorkspaceEvent::TransientStatus(
                rust_i18n::t!("git.action.pulling").to_string(),
            ));
        }
        cx.notify();
    }

    /// 请求外壳打开当前仓库的推送弹窗。
    pub(crate) fn open_push_modal(&mut self, cx: &mut Context<Self>) {
        self.push_branch(None, cx);
    }

    /// 推送指定分支或当前默认上游。
    pub(crate) fn push_branch(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenPushModal {
            project: self.state.project.clone(),
            target,
        });
    }

    /// 请求打开新建分支命名弹窗。
    pub(crate) fn create_branch_from(&mut self, start: Option<String>, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenNameModal {
            project: self.state.project.clone(),
            purpose: GitNamePurpose::CreateBranch { start },
            initial: String::new(),
        });
    }

    /// 请求打开打标签命名弹窗。
    pub(crate) fn create_tag_from(&mut self, oid: &str, cx: &mut Context<Self>) {
        let suggestion = format!("tag-{}", oid.chars().take(7).collect::<String>());
        cx.emit(GitWorkspaceEvent::OpenNameModal {
            project: self.state.project.clone(),
            purpose: GitNamePurpose::CreateTag {
                target: oid.to_string(),
            },
            initial: suggestion,
        });
    }

    /// 请求打开重命名分支命名弹窗。
    pub(crate) fn rename_branch(&mut self, old: String, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenNameModal {
            project: self.state.project.clone(),
            purpose: GitNamePurpose::RenameBranch { old: old.clone() },
            initial: old,
        });
    }

    /// 检出分支或标签/提交。
    pub(crate) fn checkout_ref(&mut self, name: &str, cx: &mut Context<Self>) {
        self.send(GitCommand::Checkout {
            project: self.state.project.clone(),
            target: GitCheckout::Branch(name.to_string()),
        });
        cx.notify();
    }

    /// 删除分支或标签（`refs/tags/` 前缀或短名）。
    pub(crate) fn delete_ref(&mut self, name: &str, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        if let Some(tag) = name.strip_prefix("refs/tags/") {
            self.send(GitCommand::DeleteTag {
                project,
                name: tag.to_string(),
            });
        } else {
            self.send(GitCommand::DeleteBranch {
                project,
                name: name.to_string(),
                force: false,
            });
        }
        cx.notify();
    }

    /// 请求打开新建工作树弹窗。
    pub(crate) fn open_worktree_modal(&mut self, start: String, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenWorktreeModal {
            project: self.state.project.clone(),
            start,
        });
    }

    /// 锁定或解锁工作树。
    pub(crate) fn set_worktree_locked(
        &mut self,
        path: String,
        locked: bool,
        cx: &mut Context<Self>,
    ) {
        self.send(if locked {
            GitCommand::LockWorktree {
                project: self.state.project.clone(),
                path,
                reason: None,
            }
        } else {
            GitCommand::UnlockWorktree {
                project: self.state.project.clone(),
                path,
            }
        });
        cx.notify();
    }

    /// 移除指定工作树。
    pub(crate) fn remove_worktree(&mut self, path: String, force: bool, cx: &mut Context<Self>) {
        self.send(GitCommand::RemoveWorktree {
            project: self.state.project.clone(),
            path,
            force,
        });
        cx.notify();
    }

    /// 请求在工作区打开工作树目录。
    pub(crate) fn open_worktree_path(&mut self, path: String, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenWorktreePath(path));
    }

    /// 请求打开交互式变基弹窗。
    pub(crate) fn open_rebase_modal(&mut self, upstream: String, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenRebaseModal {
            project: self.state.project.clone(),
            upstream,
        });
    }

    /// 请求打开冲突解决弹窗。
    pub(crate) fn open_conflict_modal(&mut self, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenConflictModal(
            self.state.project.clone(),
        ));
    }

    /// 执行历史改写与整合动作（Reset / Revert / CherryPick / Merge / Rebase）。
    pub(crate) fn history_action(&mut self, action: GitHistoryAction, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        let command = match action {
            GitHistoryAction::Reset { target, kind } => GitCommand::ResetTo {
                project,
                target,
                kind,
            },
            GitHistoryAction::Revert(oid) => GitCommand::Revert { project, oid },
            GitHistoryAction::CherryPick(oid) => GitCommand::CherryPick { project, oid },
            GitHistoryAction::Merge(reference) => GitCommand::Merge { project, reference },
            GitHistoryAction::Rebase(upstream) => GitCommand::RebaseOnto { project, upstream },
            GitHistoryAction::RebaseAbort => GitCommand::RebaseAbort { project },
        };
        self.send(command);
        cx.notify();
    }

    /// 把分支头提交压进它的父提交：`squash` 保留两条说明，否则只留父提交说明。
    pub(crate) fn fold_tip_into_parent(&mut self, squash: bool, cx: &mut Context<Self>) {
        let mode = if squash {
            GitFoldMode::Squash
        } else {
            GitFoldMode::Fixup
        };
        self.send(GitCommand::FoldTipIntoParent {
            project: self.state.project.clone(),
            mode,
        });
        cx.notify();
    }

    /// 「推送此前所有提交」：把选中之前的历史推到该分支的上游远端。
    pub(crate) fn push_up_to(&mut self, oid: &str, cx: &mut Context<Self>) {
        let branch = self.state.status
            .as_ref()
            .and_then(|status| match &status.head {
                HeadState::OnBranch(name) => Some(name.clone()),
                _ => None,
            });
        let Some(branch) = branch else {
            cx.emit(GitWorkspaceEvent::TransientStatus(
                rust_i18n::t!("git.push.no_branch").to_string(),
            ));
            cx.notify();
            return;
        };
        self.send(GitCommand::PushUpTo {
            project: self.state.project.clone(),
            oid: oid.to_string(),
            branch,
        });
        cx.notify();
    }
}
