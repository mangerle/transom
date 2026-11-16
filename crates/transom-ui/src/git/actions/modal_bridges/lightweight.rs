//! 轻量 Git 实体弹窗（名称录入、新建工作树）与 AppShell 的生命周期与事件桥接。

use crate::app::{AppShell, AppState};
use gpui_kit::*;
use transom_core::{GitCommand, GitProject};

/// 打开 Git 名称录入对话框实体。
pub(crate) fn open_git_name_modal(
    app_state: &mut AppState,
    project: &GitProject,
    purpose: crate::git::GitNamePurpose,
    default: String,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    open_name_modal_with(
        app_state,
        Some(project.clone()),
        purpose,
        default,
        window,
        cx,
    );
}

/// 打开「新建分组」名称对话框。
pub(crate) fn open_git_folder_modal(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    open_name_modal_with(
        app_state,
        None,
        crate::git::GitNamePurpose::CreateFolder,
        String::new(),
        window,
        cx,
    );
}

/// 打开「重命名分组」名称对话框。
pub(crate) fn open_git_rename_folder_modal(
    app_state: &mut AppState,
    id: transom_core::GitFolderId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(current) = app_state
        .git_registry(cx)
        .folders
        .iter()
        .find(|folder| folder.id == id)
        .map(|folder| folder.name.clone())
    else {
        return;
    };
    open_name_modal_with(
        app_state,
        None,
        crate::git::GitNamePurpose::RenameFolder { id },
        current,
        window,
        cx,
    );
}

fn open_name_modal_with(
    app_state: &mut AppState,
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
                handle_name_modal_submit(
                    &mut shell.state,
                    project.clone(),
                    purpose.clone(),
                    name.clone(),
                    cx,
                );
            }
            crate::git::GitNameModalEvent::Close => close_git_name_modal(&mut shell.state, cx),
        },
    );
    app_state.ui.modals.git.name = Some(modal);
    app_state.runtime.tasks.subscriptions.git_name_modal = Some(subscription);
    cx.notify();
}

fn handle_name_modal_submit(
    app_state: &mut AppState,
    project: Option<GitProject>,
    purpose: crate::git::GitNamePurpose,
    name: String,
    cx: &mut Context<AppShell>,
) {
    close_git_name_modal(app_state, cx);
    match purpose {
        crate::git::GitNamePurpose::CreateBranch { start } => {
            if let Some(project) = project {
                app_state.send_command(GitCommand::CreateBranch {
                    project,
                    name,
                    start,
                    switch_to: true,
                });
            }
        }
        crate::git::GitNamePurpose::CreateTag { target } => {
            if let Some(project) = project {
                app_state.send_command(GitCommand::CreateTag {
                    project,
                    name,
                    target,
                    message: None,
                });
            }
        }
        crate::git::GitNamePurpose::RenameBranch { old } => {
            if let Some(project) = project {
                app_state.send_command(GitCommand::RenameBranch {
                    project,
                    old,
                    new: name,
                });
            }
        }
        crate::git::GitNamePurpose::CreateFolder => {
            app_state.update_git_registry(cx, |git| git.add_folder(name));
            crate::git::actions::GitActions::new(app_state).persist_git_projects(cx);
        }
        crate::git::GitNamePurpose::RenameFolder { id } => {
            if app_state.update_git_registry(cx, |git| git.rename_folder(id, name)) {
                crate::git::actions::GitActions::new(app_state).persist_git_projects(cx);
            }
        }
    }
    cx.notify();
}

/// 触发名称对话框提交。
pub(crate) fn submit_git_name_modal(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    if let Some(modal) = app_state.ui.modals.git.name.as_ref() {
        modal.update(cx, |modal, cx| modal.submit(cx));
    }
}

/// 关闭名称对话框。
pub(crate) fn close_git_name_modal(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    app_state.ui.modals.git.name = None;
    app_state.runtime.tasks.subscriptions.git_name_modal = None;
    cx.notify();
}

/// 打开「新建工作树」对话框实体。
pub(crate) fn open_git_worktree_modal(
    app_state: &mut AppState,
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
                close_git_worktree_modal(&mut shell.state, cx);
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
                    shell.state.send_command(GitCommand::AddWorktree {
                        project: project.clone(),
                        path: path.clone(),
                        branch: branch.clone(),
                        start: Some(start.clone()),
                    });
                }
            }
            crate::git::GitWorktreeModalEvent::Close => {
                close_git_worktree_modal(&mut shell.state, cx)
            }
        },
    );
    app_state.ui.modals.git.worktree = Some(modal);
    app_state.runtime.tasks.subscriptions.git_worktree_modal = Some(subscription);
    cx.notify();
}

/// 触发工作树对话框提交。
pub(crate) fn submit_git_worktree_modal(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    if let Some(modal) = app_state.ui.modals.git.worktree.as_ref() {
        modal.update(cx, |modal, cx| modal.submit(cx));
    }
}

/// 关闭工作树对话框。
pub(crate) fn close_git_worktree_modal(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    app_state.ui.modals.git.worktree = None;
    app_state.runtime.tasks.subscriptions.git_worktree_modal = None;
    cx.notify();
}
