//! Git 弹窗的判定与文本项。
use crate::app::AppShell;
use gpui_kit::App;

pub(in crate::app::dialog_host) fn git_project_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.project.is_some()
}

pub(in crate::app::dialog_host) fn git_project_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .git
        .project
        .as_ref()
        .map(|modal| vec![modal.picker.path.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_clone_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.clone.is_some()
}

pub(in crate::app::dialog_host) fn git_clone_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .git
        .clone
        .as_ref()
        .map(|modal| {
            vec![
                modal.url.read(cx).value().to_string(),
                modal.picker.path.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_worktree_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.worktree.is_some()
}

pub(in crate::app::dialog_host) fn git_worktree_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .git
        .worktree
        .as_ref()
        .map(|modal| {
            vec![
                modal.branch.read(cx).value().to_string(),
                modal.path.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_name_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.name.is_some()
}

pub(in crate::app::dialog_host) fn git_name_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .git
        .name
        .as_ref()
        .map(|modal| vec![modal.input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_rebase_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.rebase.is_some()
}

pub(in crate::app::dialog_host) fn git_push_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.push.is_some()
}

pub(in crate::app::dialog_host) fn git_conflict_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.git.conflict.is_some()
}
