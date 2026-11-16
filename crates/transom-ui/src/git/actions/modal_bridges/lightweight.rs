//! 轻量 Git 实体弹窗（名称录入、新建工作树）与 AppShell 的生命周期与事件桥接。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GitCommand, GitProject};

/// 打开 Git 名称录入对话框实体。
pub(crate) fn open_git_name_modal(
    shell: &mut AppShell,
    project: &GitProject,
    purpose: crate::git::GitNamePurpose,
    default: String,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    open_name_modal_with(shell, Some(project.clone()), purpose, default, window, cx);
}

/// 打开「新建分组」名称对话框。
pub(crate) fn open_git_folder_modal(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    open_name_modal_with(
        shell,
        None,
        crate::git::GitNamePurpose::CreateFolder,
        String::new(),
        window,
        cx,
    );
}

/// 打开「重命名分组」名称对话框。
pub(crate) fn open_git_rename_folder_modal(
    shell: &mut AppShell,
    id: transom_core::GitFolderId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(current) = shell
        .state
        .git
        .folders
        .iter()
        .find(|folder| folder.id == id)
        .map(|folder| folder.name.clone())
    else {
        return;
    };
    open_name_modal_with(
        shell,
        None,
        crate::git::GitNamePurpose::RenameFolder { id },
        current,
        window,
        cx,
    );
}

fn open_name_modal_with(
    shell: &mut AppShell,
    project: Option<GitProject>,
    purpose: crate::git::GitNamePurpose,
    default: String,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let modal =
        cx.new(|cx| crate::git::GitNameModalView::new(project, purpose, &default, window, cx));
    let subscription = cx.subscribe(
        &modal,
        |shell, _, event: &crate::git::GitNameModalEvent, cx| match event {
            crate::git::GitNameModalEvent::Submit {
                project,
                purpose,
                name,
            } => {
                handle_name_modal_submit(shell, project.clone(), purpose.clone(), name.clone(), cx);
            }
            crate::git::GitNameModalEvent::Close => close_git_name_modal(shell, cx),
        },
    );
    shell.state.modals.git.name = Some(modal);
    shell.state.tasks.subscriptions.git_name_modal = Some(subscription);
    cx.notify();
}

fn handle_name_modal_submit(
    shell: &mut AppShell,
    project: Option<GitProject>,
    purpose: crate::git::GitNamePurpose,
    name: String,
    cx: &mut Context<AppShell>,
) {
    close_git_name_modal(shell, cx);
    match purpose {
        crate::git::GitNamePurpose::CreateBranch { start } => {
            if let Some(project) = project {
                shell.send_command(GitCommand::CreateBranch {
                    project,
                    name,
                    start,
                    switch_to: true,
                });
            }
        }
        crate::git::GitNamePurpose::CreateTag { target } => {
            if let Some(project) = project {
                shell.send_command(GitCommand::CreateTag {
                    project,
                    name,
                    target,
                    message: None,
                });
            }
        }
        crate::git::GitNamePurpose::RenameBranch { old } => {
            if let Some(project) = project {
                shell.send_command(GitCommand::RenameBranch {
                    project,
                    old,
                    new: name,
                });
            }
        }
        crate::git::GitNamePurpose::CreateFolder => {
            shell.state.git.add_folder(name);
            crate::git::actions::persist_git_projects(shell, cx);
        }
        crate::git::GitNamePurpose::RenameFolder { id } => {
            if shell.state.git.rename_folder(id, name) {
                crate::git::actions::persist_git_projects(shell, cx);
            }
        }
    }
    cx.notify();
}

/// 触发名称对话框提交。
pub(crate) fn submit_git_name_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    if let Some(modal) = shell.state.modals.git.name.as_ref() {
        modal.update(cx, |modal, cx| modal.submit(cx));
    }
}

/// 关闭名称对话框。
pub(crate) fn close_git_name_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.modals.git.name = None;
    shell.state.tasks.subscriptions.git_name_modal = None;
    cx.notify();
}

/// 打开「新建工作树」对话框实体。
pub(crate) fn open_git_worktree_modal(
    shell: &mut AppShell,
    project: GitProject,
    start: &str,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let modal = cx.new(|cx| crate::git::GitWorktreeModalView::new(project, start, window, cx));
    let subscription = cx.subscribe(
        &modal,
        |shell, _, event: &crate::git::GitWorktreeModalEvent, cx| match event {
            crate::git::GitWorktreeModalEvent::Submit {
                project,
                path,
                branch,
                start,
                checkout,
            } => {
                close_git_worktree_modal(shell, cx);
                if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
                    workspace.update(cx, |ws, cx| {
                        ws.create_worktree(
                            crate::git::actions::CreateWorktreeParams {
                                path: path.clone(),
                                branch: branch.clone(),
                                start: Some(start.clone()),
                                checkout: *checkout,
                            },
                            cx,
                        );
                    });
                } else {
                    shell.send_command(GitCommand::AddWorktree {
                        project: project.clone(),
                        path: path.clone(),
                        branch: branch.clone(),
                        start: Some(start.clone()),
                    });
                }
            }
            crate::git::GitWorktreeModalEvent::Close => close_git_worktree_modal(shell, cx),
        },
    );
    shell.state.modals.git.worktree = Some(modal);
    shell.state.tasks.subscriptions.git_worktree_modal = Some(subscription);
    cx.notify();
}

/// 触发工作树对话框提交。
pub(crate) fn submit_git_worktree_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    if let Some(modal) = shell.state.modals.git.worktree.as_ref() {
        modal.update(cx, |modal, cx| modal.submit(cx));
    }
}

/// 关闭工作树对话框。
pub(crate) fn close_git_worktree_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.modals.git.worktree = None;
    shell.state.tasks.subscriptions.git_worktree_modal = None;
    cx.notify();
}
