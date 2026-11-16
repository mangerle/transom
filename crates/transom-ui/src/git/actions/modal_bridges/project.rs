//! 「打开项目」与 Git 分组字段在 AppShell 侧的生命周期及事件桥接。

use crate::app::AppShell;
use crate::git::form::GitProjectDraft;
use crate::git::project_modal::{GitProjectModalEvent, GitProjectModalView};
use gpui_kit::*;
use transom_core::{GitAppEvent, GitCommand, GitFolder, GitFolderId, GitRequestId};

/// 打开「打开项目」弹窗实体并订阅探测/提交/关闭事件。
pub(crate) fn open_git_project_modal(
    shell: &mut AppShell,
    folder: Option<GitFolderId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let folders = git_folder_names(&shell.state.git_registry(cx).folders);
    let preset_name = folder.and_then(|id| {
        shell
            .state
            .git_registry(cx)
            .folders
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.name.clone())
    });
    let modal = cx.new(|cx| GitProjectModalView::new(folders, preset_name, window, cx));
    let subscription = cx.subscribe(&modal, |shell, _, event: &GitProjectModalEvent, cx| {
        on_project_modal_event(shell, event, cx);
    });
    shell.state.modals.git.project = Some(modal.clone());
    shell.state.tasks.subscriptions.git_project_modal = Some(subscription);
    let focus = modal.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    modal.update(cx, |view, cx| view.trigger_probe(cx));
    cx.notify();
}

/// 关闭「打开项目」弹窗。
pub(crate) fn close_git_project_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.modals.git.project = None;
    shell.state.tasks.subscriptions.git_project_modal = None;
    cx.notify();
}

/// 由全局快捷键或遮罩层触发当前「打开项目」弹窗的提交。
pub(crate) fn commit_git_project_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let Some(modal) = shell.state.modals.git.project.clone() else {
        return;
    };
    modal.update(cx, |view, cx| view.commit(cx));
}

fn on_project_modal_event(
    shell: &mut AppShell,
    event: &GitProjectModalEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitProjectModalEvent::Probe { request_id, path } => {
            shell.send_command(GitCommand::Probe {
                request_id: *request_id,
                path: path.clone(),
            });
        }
        GitProjectModalEvent::Commit {
            path,
            folder_name,
            icon,
            color,
            already_repo,
            init_if_missing,
        } => {
            let mut draft = GitProjectDraft::new(path.clone());
            draft.icon = *icon;
            draft.color = *color;
            draft.folder_id = git_folder_id_for_name(shell, folder_name, cx);
            if !already_repo && *init_if_missing {
                let request_id = GitRequestId::new();
                if let Some(modal) = shell.state.modals.git.project.clone() {
                    modal.update(cx, |view, cx| view.begin_init(request_id, cx));
                }
                shell
                    .state
                    .update_git_registry(cx, |git| git.pending_init = Some((request_id, draft)));
                shell.send_command(GitCommand::Init {
                    request_id,
                    path: path.clone(),
                });
            } else {
                crate::git::actions::add_git_project(shell, draft, cx);
                close_git_project_modal(shell, cx);
            }
            cx.notify();
        }
        GitProjectModalEvent::Close => close_git_project_modal(shell, cx),
    }
}

/// 将后台探测与初始化结果回填到弹窗实体。
pub(crate) fn apply_git_probe_event(
    shell: &mut AppShell,
    event: GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitAppEvent::Probed {
            request_id,
            path,
            info,
            error,
        } => {
            let Some(modal) = shell.state.modals.git.project.clone() else {
                return;
            };
            let err_msg = error.as_ref().map(crate::palette::localized_error);
            let detected = info.map(|info| *info);
            if detected.is_none() && err_msg.is_none() {
                log::debug!(
                    "目录不是 Git 仓库，等待用户选择初始化，路径: {}",
                    path.display()
                );
            }
            modal.update(cx, |view, cx| {
                view.apply_probe_result(request_id, detected, err_msg, cx);
            });
        }
        GitAppEvent::Inited {
            request_id,
            path,
            error,
        } => {
            let Some((pending_id, draft)) = shell
                .state
                .update_git_registry(cx, |git| git.pending_init.take())
            else {
                return;
            };
            if pending_id != request_id {
                shell.state.update_git_registry(cx, |git| {
                    git.pending_init = Some((pending_id, draft));
                });
                return;
            }
            match error {
                Some(error) => {
                    let message = crate::palette::localized_error(&error);
                    log::error!(
                        "初始化 Git 仓库失败，路径: {}，原因: {message}",
                        path.display()
                    );
                    if let Some(modal) = shell.state.modals.git.project.clone() {
                        modal.update(cx, |view, cx| {
                            view.apply_init_error(request_id, message, cx)
                        });
                    }
                }
                None => {
                    crate::git::actions::add_git_project(shell, draft, cx);
                    close_git_project_modal(shell, cx);
                }
            }
            cx.notify();
        }
        _ => {}
    }
}

/// 分组候选：既有分组的名字。
pub(crate) fn git_folder_names(folders: &[GitFolder]) -> Vec<String> {
    folders.iter().map(|folder| folder.name.clone()).collect()
}

/// 把输入框里的组名落成真实分组：空 → 不归组；已有同名 → 用它；
/// 新名字 → 现在就建组（确认时才建，打字过程中不落盘）。
pub(crate) fn git_folder_id_for_name(
    shell: &mut AppShell,
    name: &str,
    cx: &mut Context<AppShell>,
) -> Option<GitFolderId> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(existing) =
        crate::git::helpers::find_folder_by_name(&shell.state.git_registry(cx).folders, name)
    {
        return Some(existing);
    }
    log::info!("按表单输入新建 Git 分组: {name}");
    let created = shell.state.update_git_registry(cx, |git| {
        git.folders.push(GitFolder::new(name));
        git.folders.last().map(|folder| folder.id)
    });
    crate::git::actions::persist_git_projects(shell, cx);
    created
}
