//! Git 弹窗的判定与文本项。
use super::super::DialogCheckState;
use gpui_kit::App;

pub(in crate::app::dialog_host) fn git_project_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.project.is_some()
}

pub(in crate::app::dialog_host) fn git_project_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .modals
        .git
        .project
        .as_ref()
        .map(|modal| {
            vec![
                modal
                    .read(cx)
                    .picker
                    .read(cx)
                    .path
                    .read(cx)
                    .value()
                    .to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_clone_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.clone.is_some()
}

pub(in crate::app::dialog_host) fn git_clone_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .modals
        .git
        .clone
        .as_ref()
        .map(|modal| {
            let modal = modal.read(cx);
            vec![
                modal.url.read(cx).value().to_string(),
                modal.picker.read(cx).path.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_worktree_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.worktree.is_some()
}

pub(in crate::app::dialog_host) fn git_worktree_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .modals
        .git
        .worktree
        .as_ref()
        .map(|modal| {
            let modal = modal.read(cx);
            vec![
                modal.branch.read(cx).value().to_string(),
                modal.path.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_name_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.name.is_some()
}

pub(in crate::app::dialog_host) fn git_name_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .modals
        .git
        .name
        .as_ref()
        .map(|modal| vec![modal.read(cx).input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn git_rebase_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.rebase.is_some()
}

pub(in crate::app::dialog_host) fn git_push_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.push.is_some()
}

pub(in crate::app::dialog_host) fn git_conflict_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.git.conflict.is_some()
}
