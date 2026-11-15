//! 工作树与搁置（stash）的动作。
//!
//! 设计原理（Why）：两者都是「项目之外的对象」——工作树是另一个检出目录，
//! 搁置是栈里的快照，交互上都需要展开/刷新这类列表动作，放一起成模块。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GitCommand, GitProject, GitRequestId};

use super::GitRefAction;

/// 拉取工作树列表。
pub(crate) fn git_reload_worktrees(
    shell: &mut AppShell,
    project: &GitProject,
    cx: &mut Context<AppShell>,
) {
    let request_id = GitRequestId::new();
    if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
        workspace.update(cx, |workspace, _| {
            workspace.state.pending.push(request_id);
        });
    }
    shell.send_command(GitCommand::ListWorktrees {
        project: project.clone(),
        request_id,
    });
    cx.notify();
}

/// 按对话框填好的参数新建工作树。
pub(crate) fn git_create_worktree(
    shell: &mut AppShell,
    project: &GitProject,
    path: String,
    branch: String,
    start: Option<String>,
    checkout: bool,
    cx: &mut Context<AppShell>,
) {
    // 目录可能是相对仓库父目录的简写：补成绝对路径再交给 git，
    // 否则子进程的工作目录决定最终落点，用户看到的和实际创建的位置会偏。
    let target = {
        let candidate = std::path::PathBuf::from(&path);
        if candidate.is_absolute() {
            candidate
        } else {
            project
                .path
                .parent()
                .map(|dir| dir.join(&candidate))
                .unwrap_or(candidate)
        }
    };
    let text = target.display().to_string();
    if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
        workspace.update(cx, |workspace, _| {
            workspace.state.pending_worktree = Some((text.clone(), checkout));
        });
    }
    super::ref_actions::git_ref_action(
        shell,
        project,
        GitRefAction::NewWorktree {
            path: text,
            branch,
            start,
        },
        cx,
    );
}

/// 把一个工作树目录加进项目列表（IDEA 里点工作树就是切过去看）。
pub(crate) fn git_open_worktree_path(shell: &mut AppShell, path: &str, cx: &mut Context<AppShell>) {
    let target = std::path::PathBuf::from(path);
    if let Some(project) = shell
        .state
        .git
        .projects
        .iter()
        .find(|item| item.path == target)
        .cloned()
    {
        let tab_id = crate::git::actions::git_tab_for_project(shell, project.id, cx);
        if let Some(tab_id) = tab_id {
            let _ = tab_id;
        }
        crate::git::actions::git_refresh(shell, &project, cx);
        return;
    }
    crate::git::actions::add_git_project(shell, super::super::GitProjectDraft::new(target), cx);
}
