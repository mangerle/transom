//! 「克隆仓库」弹窗在 AppShell 侧的生命周期及事件桥接。

use super::project::git_folder_names;
use crate::app::{AppShell, AppState};
use crate::git::clone_modal::{GitCloneModalEvent, GitCloneModalView, GitCloneTask};
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId};

/// 打开「克隆仓库」弹窗实体并订阅启动/关闭事件。
pub(crate) fn open_git_clone_modal(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let folders = git_folder_names(&app_state.git_registry(cx).folders);
    let modal = cx.new(|cx| GitCloneModalView::new(folders, window, cx));
    let subscription = cx.subscribe(&modal, |shell, _, event: &GitCloneModalEvent, cx| {
        on_clone_modal_event(&mut shell.state, event, cx);
    });
    app_state.ui.modals.git.clone = Some(modal.clone());
    app_state.runtime.tasks.subscriptions.git_clone_modal = Some(subscription);
    let focus = modal.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    cx.notify();
}

/// 关闭「克隆仓库」弹窗。克隆一旦开始就由进度卡接管，这里不取消在途任务。
pub(crate) fn close_git_clone_modal(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    app_state.ui.modals.git.clone = None;
    app_state.runtime.tasks.subscriptions.git_clone_modal = None;
    cx.notify();
}

/// 由全局快捷键触发当前「克隆仓库」弹窗的提交。
pub(crate) fn submit_git_clone_modal(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    let Some(modal) = app_state.ui.modals.git.clone.clone() else {
        return;
    };
    modal.update(cx, |view, cx| view.submit(cx));
}

fn on_clone_modal_event(
    app_state: &mut AppState,
    event: &GitCloneModalEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitCloneModalEvent::Start {
            url,
            dest,
            depth,
            repo_name,
            folder_name,
            icon,
            color,
        } => {
            // 一次只跑一个克隆：覆盖任务表会让旧子进程失去取消入口。
            if app_state.git_registry(cx).clone_task.is_some() {
                if let Some(modal) = app_state.ui.modals.git.clone.clone() {
                    let message = rust_i18n::t!("git.clone.busy").to_string();
                    modal.update(cx, |view, cx| view.set_error(message, cx));
                }
                cx.notify();
                return;
            }
            let request_id = GitRequestId::new();
            app_state.update_git_registry(cx, |git| {
                git.clone_task = Some(GitCloneTask {
                    request_id,
                    repo_name: repo_name.clone(),
                    folder_name: folder_name.clone(),
                    icon: *icon,
                    color: *color,
                    last_line: String::new(),
                    percent: None,
                    running: true,
                    error: None,
                    done: None,
                });
            });
            close_git_clone_modal(app_state, cx);
            app_state.send_command(GitCommand::Clone {
                request_id,
                url: url.clone(),
                dest: dest.clone(),
                depth: *depth,
            });
            cx.notify();
        }
        GitCloneModalEvent::Close => close_git_clone_modal(app_state, cx),
    }
}
