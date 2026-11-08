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
                shell.close_git_project_modal(cx);
                Some(true)
            }
            ModalKind::GitClone => {
                shell.close_git_clone_modal(cx);
                Some(true)
            }
            ModalKind::GitWorktree => {
                shell.close_git_worktree_modal(cx);
                Some(true)
            }
            ModalKind::GitName => {
                shell.close_git_name_modal(cx);
                Some(true)
            }
            ModalKind::GitRebase => {
                shell.git_close_rebase_modal(cx);
                Some(true)
            }
            ModalKind::GitPush => {
                shell.close_git_push_modal(cx);
                Some(true)
            }
            ModalKind::GitConflict => {
                shell.close_git_conflict_modal(cx);
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
                shell.commit_git_project_modal(cx);
                Some(())
            }
            ModalKind::GitClone => {
                shell.start_git_clone(window, cx);
                Some(())
            }
            ModalKind::GitWorktree => {
                shell.commit_git_worktree(cx);
                Some(())
            }
            ModalKind::GitName => {
                shell.submit_git_name(cx);
                Some(())
            }
            ModalKind::GitRebase => {
                shell.git_rebase_run(cx);
                Some(())
            }
            ModalKind::GitPush => {
                shell.submit_git_push_modal(cx);
                Some(())
            }
            ModalKind::GitConflict => {
                shell.close_git_conflict_modal(cx);
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
            ModalKind::GitWorktree => shell
                .state
                .modals
                .git
                .worktree
                .as_ref()
                .map(|modal| ModalFocus::Input(modal.branch.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::GitName => shell
                .state
                .modals
                .git
                .name
                .as_ref()
                .map(|modal| ModalFocus::Input(modal.input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::GitRebase | ModalKind::GitPush | ModalKind::GitConflict => ModalFocus::None,
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
            ModalKind::GitProject => shell.render_git_project_dialog_body(app, window, cx, p),
            ModalKind::GitClone => shell.render_git_clone_dialog_body(app, window, cx, p),
            ModalKind::GitWorktree => shell.render_git_worktree_dialog_body(app, window, cx, p),
            ModalKind::GitName => shell.render_git_name_dialog_body(app, cx, p),
            ModalKind::GitRebase => shell.render_git_rebase_dialog_body(app, window, cx, p),
            ModalKind::GitPush => shell.render_git_push_dialog_body(app, window, cx, p),
            ModalKind::GitConflict => shell.render_git_conflict_dialog_body(app, window, cx, p),
            _ => return None,
        };
        Some(element)
    }
}
