//! Git 仓库管理、克隆与分支操作模态弹窗的状态判定、提交、取消、焦点与渲染分发。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// Git 弹窗是否处于打开状态。若属于 Git 弹窗种类，返回 `Some(bool)`；否则返回 `None`。
    pub(super) fn git_is_open(self, shell: &AppShell) -> Option<bool> {
        let is_open = match self {
            ModalKind::GitProject => shell.state.modals.git.project.is_some(),
            ModalKind::GitClone => shell.state.modals.git.clone.is_some(),
            ModalKind::GitWorktree => shell.state.modals.git.worktree.is_some(),
            ModalKind::GitName => shell.state.modals.git.name.is_some(),
            ModalKind::GitRebase => shell.state.modals.git.rebase.is_some(),
            ModalKind::GitPush => shell.state.modals.git.push.is_some(),
            ModalKind::GitConflict => shell.state.modals.git.conflict.is_some(),
            _ => return None,
        };
        Some(is_open)
    }

    /// Git 弹窗的跟踪文本项（用于表单脏检查拦截）。
    pub(super) fn git_field_values(self, shell: &AppShell, cx: &App) -> Option<Vec<String>> {
        let values = match self {
            ModalKind::GitProject => shell
                .state
                .modals
                .git
                .project
                .as_ref()
                .map(|modal| vec![modal.picker.path.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::GitClone => shell
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
                .unwrap_or_default(),
            ModalKind::GitWorktree => shell
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
                .unwrap_or_default(),
            ModalKind::GitName => shell
                .state
                .modals
                .git
                .name
                .as_ref()
                .map(|modal| vec![modal.input.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::GitRebase | ModalKind::GitPush | ModalKind::GitConflict => Vec::new(),
            _ => return None,
        };
        Some(values)
    }

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

    /// Git 弹窗尺寸规格与唯一标识。
    pub(super) fn git_spec(self, window: &Window) -> Option<super::ModalSpec> {
        let rem = window.rem_size();
        let (px_width, slug) = match self {
            ModalKind::GitProject => (Some(crate::theme::tokens::dialog::GIT_FORM), "git-project"),
            ModalKind::GitClone => (Some(crate::theme::tokens::dialog::GIT_CLONE), "git-clone"),
            ModalKind::GitWorktree => {
                (Some(crate::theme::tokens::dialog::GIT_FORM), "git-worktree")
            }
            ModalKind::GitName => (Some(crate::theme::tokens::dialog::INPUT), "git-name"),
            ModalKind::GitRebase => (Some(crate::theme::tokens::dialog::GIT_FORM), "git-rebase"),
            ModalKind::GitPush => (Some(crate::theme::tokens::dialog::GIT_PUSH), "git-push"),
            ModalKind::GitConflict => (
                Some(crate::theme::tokens::dialog::GIT_CONFLICT),
                "git-conflict",
            ),
            _ => return None,
        };

        let width = px_width.map_or(
            rem * crate::theme::tokens::size::DIALOG_MAX_WIDTH,
            |width| rem * width,
        );
        Some(super::ModalSpec { width, slug })
    }
}
