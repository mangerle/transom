//! 引用与远端动作：分支/标签的增删切、提交、合并变基撤销、抓取推送。
//!
//! 设计原理（Why）：这一组全是写动作，命令构造与 `GitAction` 一一对应，
//! 集中放置便于逐项核对「哪些动作会改写历史」，也避免每个动作各写一遍项目定位逻辑。

use crate::app::AppShell;
use crate::git::GitNamePurpose;
use gpui_kit::*;
use transom_core::{GitCommand, GitProject};

/// 提交（模态框调用）。
pub(crate) fn git_commit(
    shell: &mut AppShell,
    project: &GitProject,
    message: String,
    amend: bool,
    push_after: bool,
    cx: &mut Context<AppShell>,
) {
    if message.trim().is_empty() && !amend {
        shell.set_error(rust_i18n::t!("git.commit.empty_message").to_string());
        return;
    }
    let fixup_target = if amend {
        shell
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| {
                let state = &workspace.read(cx).state;
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
    shell.send_command(GitCommand::Commit {
        project: project.clone(),
        message,
        amend,
        fixup_target,
    });

    if push_after {
        // 提交成功后自动唤起推送弹窗（与在分支上右键推送展示一致）。
        if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
            workspace.update(cx, |workspace, _| {
                workspace.state.push_after_commit = true;
            });
        }
    }
    cx.notify();
}

/// 分支与标签动作。
pub(crate) fn git_ref_action(
    shell: &mut AppShell,
    project: &GitProject,
    action: GitRefAction,
    cx: &mut Context<AppShell>,
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
    shell.send_command(command);
    cx.notify();
}

/// 远程动作（更新 / 推送 / 拉取）。
pub(crate) fn git_remote_action(
    shell: &mut AppShell,
    project: &GitProject,
    action: GitRemoteAction,
    cx: &mut Context<AppShell>,
) {
    let command = match action {
        GitRemoteAction::Fetch => GitCommand::Fetch {
            project: project.clone(),
        },
        GitRemoteAction::Push => {
            let show_dialog = crate::app::app_settings(cx).git.push_dialog;
            if show_dialog {
                crate::git::push_modal::open_git_push_modal(shell, project, None, cx);
                return;
            }
            GitCommand::Push {
                project: project.clone(),
                options: transom_core::GitPushOptions::default(),
            }
        }
        GitRemoteAction::Pull { rebase } => GitCommand::Pull {
            project: project.clone(),
            rebase,
        },
    };
    shell.send_command(command);
    cx.notify();
}

/// 检出指定项目的分支或标签。
pub(crate) fn git_checkout_project_ref(
    shell: &mut AppShell,
    project: &GitProject,
    name: &str,
    cx: &mut Context<AppShell>,
) {
    git_ref_action(shell, project, GitRefAction::Checkout(name.to_string()), cx);
}

/// 删除指定项目的分支或标签：名字带 `refs/tags/` 语义时按标签删。
pub(crate) fn git_delete_project_ref(
    shell: &mut AppShell,
    project: &GitProject,
    name: &str,
    cx: &mut Context<AppShell>,
) {
    let is_tag = shell
        .state
        .git_workspace_of_project(project.id, cx)
        .is_some_and(|workspace| {
            workspace
                .read(cx)
                .state
                .tags
                .iter()
                .any(|tag| tag.name == name)
        });
    let action = if is_tag {
        GitRefAction::DeleteTag(name.to_string())
    } else {
        GitRefAction::DeleteBranch {
            name: name.to_string(),
            force: false,
        }
    };
    git_ref_action(shell, project, action, cx);
}

/// 从指定项目的某提交/引用新建分支（弹出名称输入框）。
pub(crate) fn git_create_project_branch_from(
    shell: &mut AppShell,
    project: &GitProject,
    start: &str,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let suggestion = format!("branch-{}", start.chars().take(7).collect::<String>());
    crate::git::name_modal::open_git_name_modal(
        shell,
        project,
        GitNamePurpose::CreateBranch {
            start: Some(start.to_string()),
        },
        suggestion,
        window,
        cx,
    );
}

/// 从某提交/引用新建分支（按提示名创建并切换）。
pub(crate) fn git_create_branch_from(
    shell: &mut AppShell,
    start: &str,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(project) = crate::git::helpers::git_project_of_tab(shell, None, cx) else {
        return;
    };
    git_create_project_branch_from(shell, &project, start, window, cx);
}

/// 在指定项目的某提交上打标签：弹名称框。
pub(crate) fn git_create_project_tag_from(
    shell: &mut AppShell,
    project: &GitProject,
    oid: &str,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let suggestion = format!("tag-{}", oid.chars().take(7).collect::<String>());
    crate::git::name_modal::open_git_name_modal(
        shell,
        project,
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
    shell: &mut AppShell,
    project: &GitProject,
    action: GitHistoryAction,
    cx: &mut Context<AppShell>,
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
    shell.send_command(command);
    cx.notify();
}

/// 在指定 Git 项目上执行历史改写整合动作。
pub(crate) fn git_history_on_project(
    shell: &mut AppShell,
    project: &GitProject,
    action: GitHistoryAction,
    cx: &mut Context<AppShell>,
) {
    git_history_action(shell, project, action, cx);
}

/// 在当前 Git 项目上执行历史动作：菜单回调只带得住提交号，项目由这里定位。
pub(crate) fn git_history_on_current(
    shell: &mut AppShell,
    action: GitHistoryAction,
    cx: &mut Context<AppShell>,
) {
    let Some(project) = crate::git::helpers::git_project_of_tab(shell, None, cx) else {
        shell.set_error(rust_i18n::t!("git.diff.none").to_string());
        return;
    };
    git_history_on_project(shell, &project, action, cx);
}

/// 把分支头提交压进它的父提交：`squash` 保留两条说明，否则只留父提交说明。
pub(crate) fn git_fold_tip_into_parent(
    shell: &mut AppShell,
    project: &GitProject,
    squash: bool,
    cx: &mut Context<AppShell>,
) {
    shell.send_command(GitCommand::FoldTipIntoParent {
        project: project.clone(),
        mode: if squash {
            transom_core::GitFoldMode::Squash
        } else {
            transom_core::GitFoldMode::Fixup
        },
    });
    cx.notify();
}

/// 拉取指定项目分支上游。
pub(crate) fn git_pull_project(
    shell: &mut AppShell,
    project: &GitProject,
    rebase: bool,
    cx: &mut Context<AppShell>,
) {
    git_remote_action(shell, project, GitRemoteAction::Pull { rebase }, cx);
}

/// 推送指定项目分支（根据设置决定是直接推送还是弹出推送模态弹窗）。
pub(crate) fn git_push_project(
    shell: &mut AppShell,
    project: &GitProject,
    target: Option<String>,
    cx: &mut Context<AppShell>,
) {
    let show_dialog = crate::app::app_settings(cx).git.push_dialog;
    if show_dialog {
        crate::git::push_modal::open_git_push_modal(shell, project, target, cx);
    } else {
        let options = transom_core::GitPushOptions {
            branch: target,
            ..Default::default()
        };
        shell.send_command(GitCommand::Push {
            project: project.clone(),
            options,
        });
        cx.notify();
    }
}

/// 「推送此前所有提交」：把选中之前的历史推到该分支的上游远端。
///
/// 设计原理（Why）：只有头指针挂在分支上时才谈得上「推到哪个分支」——
/// 游离头没有分支名，硬推等于在远端凭空造引用；至于是否快进，交给 git 拒绝，
/// 本层不加 `--force`，也不在客户端重复一遍祖先判断。
pub(crate) fn git_push_up_to(
    shell: &mut AppShell,
    project: &GitProject,
    oid: &str,
    cx: &mut Context<AppShell>,
) {
    let branch = shell
        .state
        .git_workspace_of_project(project.id, cx)
        .and_then(|workspace| workspace.read(cx).state.status.clone())
        .and_then(|status| match status.head {
            transom_core::HeadState::OnBranch(name) => Some(name),
            _ => None,
        });
    let Some(branch) = branch else {
        shell.set_transient_status(rust_i18n::t!("git.push.no_branch"), cx);
        cx.notify();
        return;
    };
    shell.send_command(GitCommand::PushUpTo {
        project: project.clone(),
        oid: oid.to_string(),
        branch,
    });
    cx.notify();
}

/// 刷新所有处于管理状态的 Git 项目状态（用于窗口重新聚焦时自动刷新）。
pub(crate) fn git_refresh_active_projects(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let projects = shell.state.git.projects.clone();
    for project in projects {
        crate::git::actions::git_refresh(shell, &project, cx);
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
