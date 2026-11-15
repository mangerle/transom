//! 引用与远端动作：工作树创建与多项目状态刷新。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GitCommand, GitProject};

/// 分支与标签动作（当前仅工作树创建弹窗复用）。
pub(crate) fn git_ref_action(
    shell: &mut AppShell,
    project: &GitProject,
    action: GitRefAction,
    cx: &mut Context<AppShell>,
) {
    let command = match action {
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

/// 刷新所有处于管理状态的 Git 项目状态（用于窗口重新聚焦时自动刷新）。
pub(crate) fn git_refresh_active_projects(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let projects = shell.state.git.projects.clone();
    for project in projects {
        crate::git::actions::git_refresh(shell, &project, cx);
    }
}

/// 分支/标签类动作（界面侧的意图，落到具体命令）。
pub(crate) enum GitRefAction {
    NewWorktree {
        path: String,
        branch: String,
        start: Option<String>,
    },
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
