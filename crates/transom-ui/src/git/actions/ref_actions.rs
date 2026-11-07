//! 引用与远端动作：分支/标签的增删切、提交、合并变基撤销、抓取推送。
//!
//! 设计原理（Why）：这一组全是写动作，命令构造与 `GitAction` 一一对应，
//! 集中放置便于逐项核对「哪些动作会改写历史」，也避免每个动作各写一遍项目定位逻辑。

use crate::app::AppShell;
use crate::git::GitNamePurpose;
use gpui_kit::*;
use transom_core::{GitCommand, GitProject};

impl AppShell {
    /// 提交（模态框调用）。
    pub(crate) fn git_commit(
        &mut self,
        project: &GitProject,
        message: String,
        amend: bool,
        push_after: bool,
        cx: &mut Context<Self>,
    ) {
        if message.trim().is_empty() && !amend {
            self.set_error(rust_i18n::t!("git.commit.empty_message").to_string());
            return;
        }
        let fixup_target = if amend {
            self.git_state_of(project.id).and_then(|state| {
                let target = state.amend_target.as_ref()?;
                let head_oid = state.log.entries.first().map(|e| &e.oid);
                if head_oid != Some(target) {
                    Some(target.clone())
                } else {
                    None
                }
            })
        } else {
            None
        };
        self.send_command(GitCommand::Commit {
            project: project.clone(),
            message,
            amend,
            fixup_target,
        });

        if push_after {
            // 提交成功后自动唤起推送弹窗（与在分支上右键推送展示一致）。
            if let Some(state) = self.git_state_mut(project.id) {
                state.push_after_commit = true;
            }
        }
        cx.notify();
    }

    /// 分支与标签动作。
    pub(crate) fn git_ref_action(
        &mut self,
        project: &GitProject,
        action: GitRefAction,
        cx: &mut Context<Self>,
    ) {
        let command = match action {
            GitRefAction::Checkout(branch) => GitCommand::Checkout {
                project: project.clone(),
                target: transom_core::GitCheckout::Branch(branch),
            },
            GitRefAction::DeleteBranch { name, force } => GitCommand::DeleteBranch {
                project: project.clone(),
                name,
                force,
            },
            GitRefAction::DeleteTag(name) => GitCommand::DeleteTag {
                project: project.clone(),
                name,
            },
            GitRefAction::NewWorktree {
                path,
                branch,
                start,
            } => GitCommand::AddWorktree {
                project: project.clone(),
                path,
                branch,
                start,
            },
        };
        self.send_command(command);
        cx.notify();
    }

    /// 远程动作（更新 / 推送 / 拉取）。
    pub(crate) fn git_remote_action(
        &mut self,
        project: &GitProject,
        action: GitRemoteAction,
        cx: &mut Context<Self>,
    ) {
        let command = match action {
            GitRemoteAction::Fetch => GitCommand::Fetch {
                project: project.clone(),
            },
            GitRemoteAction::Push => GitCommand::Push {
                project: project.clone(),
                branch: None,
                tags: false,
            },
            GitRemoteAction::Pull { rebase } => GitCommand::Pull {
                project: project.clone(),
                rebase,
            },
        };
        self.send_command(command);
        cx.notify();
    }

    /// 检出分支或标签。
    pub(crate) fn git_checkout_ref(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(project) = self.git_project_of_tab(None) else {
            return;
        };
        self.git_ref_action(&project, GitRefAction::Checkout(name.to_string()), cx);
    }

    /// 删除分支或标签：名字带 `refs/tags/` 语义时按标签删。
    pub(crate) fn git_delete_ref(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(project) = self.git_project_of_tab(None) else {
            return;
        };
        let is_tag = self
            .git_state_of(project.id)
            .is_some_and(|state| state.tags.iter().any(|tag| tag.name == name));
        let action = if is_tag {
            GitRefAction::DeleteTag(name.to_string())
        } else {
            GitRefAction::DeleteBranch {
                name: name.to_string(),
                force: false,
            }
        };
        self.git_ref_action(&project, action, cx);
    }

    /// 从某提交/引用新建分支（按提示名创建并切换）。
    /// 从某提交新建分支：弹名称框，不再自动生成 `branch-<短号>` 这种没法读的名字。
    pub(crate) fn git_create_branch_from(
        &mut self,
        start: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.git_project_of_tab(None) else {
            return;
        };
        let suggestion = format!("branch-{}", start.chars().take(7).collect::<String>());
        self.open_git_name_modal(
            &project,
            GitNamePurpose::CreateBranch {
                start: Some(start.to_string()),
            },
            suggestion,
            window,
            cx,
        );
    }

    /// 在某提交上打标签：同样弹名称框。
    pub(crate) fn git_create_tag_from(
        &mut self,
        oid: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.git_project_of_tab(None) else {
            return;
        };
        let suggestion = format!("tag-{}", oid.chars().take(7).collect::<String>());
        self.open_git_name_modal(
            &project,
            GitNamePurpose::CreateTag {
                target: oid.to_string(),
            },
            suggestion,
            window,
            cx,
        );
    }

    /// 历史改写类动作：重置 / 撤销 / 拣选 / 合并 / 变基 / 改写消息。
    pub(crate) fn git_history_action(
        &mut self,
        project: &GitProject,
        action: GitHistoryAction,
        cx: &mut Context<Self>,
    ) {
        let command = match action {
            GitHistoryAction::Reset { target, kind } => GitCommand::ResetTo {
                project: project.clone(),
                target,
                kind,
            },
            GitHistoryAction::Revert(oid) => GitCommand::Revert {
                project: project.clone(),
                oid,
            },
            GitHistoryAction::CherryPick(oid) => GitCommand::CherryPick {
                project: project.clone(),
                oid,
            },
            GitHistoryAction::Merge(reference) => GitCommand::Merge {
                project: project.clone(),
                reference,
            },
            GitHistoryAction::Rebase(upstream) => GitCommand::RebaseOnto {
                project: project.clone(),
                upstream,
            },
            GitHistoryAction::RebaseAbort => GitCommand::RebaseAbort {
                project: project.clone(),
            },
        };
        self.send_command(command);
        cx.notify();
    }

    /// 在当前 Git 项目上执行历史动作：菜单回调只带得住提交号，项目由这里定位。
    pub(crate) fn git_history_on_current(
        &mut self,
        action: GitHistoryAction,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.git_project_of_tab(None) else {
            self.set_error(rust_i18n::t!("git.diff.none").to_string());
            return;
        };
        self.git_history_action(&project, action, cx);
    }

    /// 把分支头提交压进它的父提交：`squash` 保留两条说明，否则只留父提交说明。
    pub(crate) fn git_fold_tip_into_parent(
        &mut self,
        project: &GitProject,
        squash: bool,
        cx: &mut Context<Self>,
    ) {
        self.send_command(GitCommand::FoldTipIntoParent {
            project: project.clone(),
            mode: if squash {
                transom_core::GitFoldMode::Squash
            } else {
                transom_core::GitFoldMode::Fixup
            },
        });
        cx.notify();
    }

    /// 拉取当前分支上游。
    pub(crate) fn git_pull_current(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.git_project_of_tab(None) else {
            return;
        };
        self.git_remote_action(&project, GitRemoteAction::Pull { rebase: false }, cx);
    }

    /// 推送当前分支（弹出对齐 openidea / IDEA 的推送模态弹窗）。
    pub(crate) fn git_push_current(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.git_project_of_tab(None) else {
            return;
        };
        self.open_git_push_modal(&project, None, cx);
    }

    /// 「推送此前所有提交」：把选中之前的历史推到该分支的上游远端。
    ///
    /// 设计原理（Why）：只有头指针挂在分支上时才谈得上「推到哪个分支」——
    /// 游离头没有分支名，硬推等于在远端凭空造引用；至于是否快进，交给 git 拒绝，
    /// 本层不加 `--force`，也不在客户端重复一遍祖先判断。
    pub(crate) fn git_push_up_to(
        &mut self,
        project: &GitProject,
        oid: &str,
        cx: &mut Context<Self>,
    ) {
        let branch = self
            .git_state_of(project.id)
            .and_then(|state| state.status.clone())
            .and_then(|status| match status.head {
                transom_core::HeadState::OnBranch(name) => Some(name),
                _ => None,
            });
        let Some(branch) = branch else {
            self.state.status = Some(rust_i18n::t!("git.push.no_branch").to_string());
            cx.notify();
            return;
        };
        self.send_command(GitCommand::PushUpTo {
            project: project.clone(),
            oid: oid.to_string(),
            branch,
        });
        cx.notify();
    }
}

/// 分支/标签类动作（界面侧的意图，落到具体命令）。
pub(crate) enum GitRefAction {
    Checkout(String),
    DeleteBranch {
        name: String,
        force: bool,
    },
    DeleteTag(String),
    NewWorktree {
        path: String,
        branch: String,
        start: Option<String>,
    },
}

/// 远程动作。
pub(crate) enum GitRemoteAction {
    Fetch,
    Push,
    Pull { rebase: bool },
}

/// 历史改写与整合动作（界面意图，落到具体命令）。
pub(crate) enum GitHistoryAction {
    Reset {
        target: String,
        kind: transom_core::GitResetKind,
    },
    Revert(String),
    CherryPick(String),
    Merge(String),
    Rebase(String),
    RebaseAbort,
}
