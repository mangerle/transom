//! Git 仓库管理、克隆与分支操作模态弹窗的状态判定、提交、取消、焦点与渲染分发。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// Git 弹窗取消操作。
    pub(super) fn git_cancel(
        self,
        shell: &mut AppShell,
        cx: &mut Context<AppShell>,
    ) -> Option<bool> {
        match self {
            ModalKind::GitProject => {
                crate::git::project_modal::close_git_project_modal(shell, cx);
                Some(true)
            }
            ModalKind::GitClone => {
                crate::git::clone_modal::close_git_clone_modal(shell, cx);
                Some(true)
            }
            ModalKind::GitWorktree => {
                crate::git::actions::modal_bridges::close_git_worktree_modal(shell, cx);
                Some(true)
            }
            ModalKind::GitName => {
                crate::git::actions::modal_bridges::close_git_name_modal(shell, cx);
                Some(true)
            }
            ModalKind::GitRebase => {
                crate::git::actions::modal_bridges::close_git_rebase_modal(shell, cx);
                Some(true)
            }
            ModalKind::GitPush => {
                crate::git::actions::modal_bridges::close_git_push_modal(shell, cx);
                Some(true)
            }
            ModalKind::GitConflict => {
                crate::git::actions::modal_bridges::close_git_conflict_modal(shell, cx);
                Some(true)
            }
            _ => None,
        }
    }

    /// Git 弹窗提交动作。若属于 Git 弹窗种类，执行提交并返回 `Some(())`；否则返回 `None`。
    pub(super) fn git_commit(
        self,
        shell: &mut AppShell,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<()> {
        match self {
            ModalKind::GitProject => {
                crate::git::project_modal::commit_git_project_modal(shell, cx);
                Some(())
            }
            ModalKind::GitClone => {
                crate::git::clone_modal::start_git_clone(shell, window, cx);
                Some(())
            }
            ModalKind::GitWorktree => {
                crate::git::actions::modal_bridges::submit_git_worktree_modal(shell, cx);
                Some(())
            }
            ModalKind::GitName => {
                crate::git::actions::modal_bridges::submit_git_name_modal(shell, cx);
                Some(())
            }
            ModalKind::GitRebase => {
                crate::git::actions::modal_bridges::run_git_rebase_modal(shell, cx);
                Some(())
            }
            ModalKind::GitPush => {
                crate::git::actions::modal_bridges::submit_git_push_modal(shell, cx);
                Some(())
            }
            ModalKind::GitConflict => {
                crate::git::actions::modal_bridges::close_git_conflict_modal(shell, cx);
                Some(())
            }
            _ => None,
        }
    }

    /// Git 弹窗初始焦点控件。
    pub(super) fn git_focus_target(self, shell: &AppShell) -> Option<ModalFocus> {
        let focus = match self {
            ModalKind::GitProject => shell
                .state
                .modals
                .git
                .project
                .as_ref()
                .map(|modal| ModalFocus::Input(modal.picker.path.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::GitClone => shell
                .state
                .modals
                .git
                .clone
                .as_ref()
                .map(|modal| ModalFocus::Input(modal.url.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::GitWorktree
            | ModalKind::GitName
            | ModalKind::GitRebase
            | ModalKind::GitPush
            | ModalKind::GitConflict => ModalFocus::None,
            _ => return None,
        };
        Some(focus)
    }

    /// Git 弹窗主体渲染分发。
    pub(super) fn render_git_dialog_body(
        self,
        shell: &AppShell,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> Option<AnyElement> {
        let element = match self {
            ModalKind::GitProject => {
                crate::git::project_modal::render_git_project_dialog_body(shell, app, window, cx, p)
            }
            ModalKind::GitClone => crate::git::clone_modal::view::render_git_clone_dialog_body(
                shell, app, window, cx, p,
            ),
            ModalKind::GitWorktree => shell
                .state
                .modals
                .git
                .worktree
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitName => shell
                .state
                .modals
                .git
                .name
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitRebase => shell
                .state
                .modals
                .git
                .rebase
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitPush => shell
                .state
                .modals
                .git
                .push
                .as_ref()
                .map(|modal| modal.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::GitConflict => shell
                .state
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
