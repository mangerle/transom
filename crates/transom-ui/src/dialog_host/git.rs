//! Git 仓库管理、克隆与分支操作模态弹窗的状态判定、提交、取消、焦点与渲染分发。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::{AppShell, AppState};
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// Git 弹窗取消操作。
    pub(super) fn git_cancel(
        self,
        app_state: &mut AppState,
        cx: &mut Context<AppShell>,
    ) -> Option<bool> {
        match self {
            ModalKind::GitProject => {
                crate::git::actions::modal_bridges::close_git_project_modal(app_state, cx);
                Some(true)
            }
            ModalKind::GitClone => {
                crate::git::actions::modal_bridges::close_git_clone_modal(app_state, cx);
                Some(true)
            }
            ModalKind::GitWorktree => {
                crate::git::actions::modal_bridges::close_git_worktree_modal(app_state, cx);
                Some(true)
            }
            ModalKind::GitName => {
                crate::git::actions::modal_bridges::close_git_name_modal(app_state, cx);
                Some(true)
            }
            ModalKind::GitRebase => {
                crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                    .close_git_rebase_modal(cx);
                Some(true)
            }
            ModalKind::GitPush => {
                crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                    .close_git_push_modal(cx);
                Some(true)
            }
            ModalKind::GitConflict => {
                crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                    .close_git_conflict_modal(cx);
                Some(true)
            }
            _ => None,
        }
    }

    /// Git 弹窗提交动作。若属于 Git 弹窗种类，执行提交并返回 `Some(())`；否则返回 `None`。
    pub(super) fn git_commit(
        self,
        app_state: &mut AppState,
        _window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<()> {
        match self {
            ModalKind::GitProject => {
                crate::git::actions::modal_bridges::commit_git_project_modal(app_state, cx);
                Some(())
            }
            ModalKind::GitClone => {
                crate::git::actions::modal_bridges::submit_git_clone_modal(app_state, cx);
                Some(())
            }
            ModalKind::GitWorktree => {
                crate::git::actions::modal_bridges::submit_git_worktree_modal(app_state, cx);
                Some(())
            }
            ModalKind::GitName => {
                crate::git::actions::modal_bridges::submit_git_name_modal(app_state, cx);
                Some(())
            }
            ModalKind::GitRebase => {
                crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                    .run_git_rebase_modal(cx);
                Some(())
            }
            ModalKind::GitPush => {
                crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                    .submit_git_push_modal(cx);
                Some(())
            }
            ModalKind::GitConflict => {
                crate::git::actions::modal_bridges::GitModalActions::new(app_state)
                    .close_git_conflict_modal(cx);
                Some(())
            }
            _ => None,
        }
    }

    /// Git 弹窗初始焦点控件。
    pub(super) fn git_focus_target(self, _app_state: &AppState) -> Option<ModalFocus> {
        match self {
            ModalKind::GitProject
            | ModalKind::GitClone
            | ModalKind::GitWorktree
            | ModalKind::GitName
            | ModalKind::GitRebase
            | ModalKind::GitPush
            | ModalKind::GitConflict => Some(ModalFocus::None),
            _ => None,
        }
    }

    /// Git 弹窗主体渲染分发。
    pub(super) fn render_git_dialog_body(
        self,
        app_state: &AppState,
        _app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        _p: &Palette,
    ) -> Option<AnyElement> {
        let element = match self {
            ModalKind::GitProject => app_state
                .ui
                .modals
                .git
                .project
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitClone => app_state
                .ui
                .modals
                .git
                .clone
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitWorktree => app_state
                .ui
                .modals
                .git
                .worktree
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitName => app_state
                .ui
                .modals
                .git
                .name
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitRebase => app_state
                .ui
                .modals
                .git
                .rebase
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitPush => app_state
                .ui
                .modals
                .git
                .push
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitConflict => app_state
                .ui
                .modals
                .git
                .conflict
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            _ => return None,
        };
        Some(element)
    }
}
