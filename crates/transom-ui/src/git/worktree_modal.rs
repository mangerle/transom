//! 「新建工作树」对话框。
//!
//! 设计原理（Why）：
//! - IDEA 的「从这里新建工作树」就是一次带三个输入项的确认：目录、分支名、
//!   起点。之前我按分支名自动派生目录再静默创建，用户既看不到落在哪、也改不了
//!   分支名，等于把一个有后果的操作藏起来了；
//! - 起点由调用方带进来（哪个分支/标签/提交），对话框里只读展示：改起点等于
//!   换了一个入口语义，不如让用户从目标那一行再点一次；
//! - 目录默认值取「仓库同级 / 仓库名-分支名」，与 IDEA 的预填一致，但保持可改。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::GitProject;

/// 新建工作树的表单状态。
pub(crate) struct GitWorktreeModal {
    pub project: GitProject,
    /// 起点引用（分支名 / 标签名 / 提交号）。
    pub start: String,
    pub path: Entity<InputState>,
    pub branch: Entity<InputState>,
    /// 创建后是否同时检出该分支（`git worktree add` 本身就会检出，这里保留
    /// 给未来的「只建不切」选项）。
    pub checkout: bool,
    pub error: Option<String>,
}

impl AppShell {
    /// 打开对话框：预填目录与分支名。
    pub(crate) fn open_git_worktree_modal(
        &mut self,
        project: GitProject,
        start: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let branch = start.replace('/', "-");
        let parent = project
            .path
            .parent()
            .map(|dir| dir.join(format!("{}-{branch}", project.display_name())))
            .unwrap_or_else(|| project.path.clone());
        self.state.modals.git.worktree = Some(GitWorktreeModal {
            project,
            start: start.to_string(),
            path: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(branch_dir_text(&branch))
                    .placeholder(rust_i18n::t!("git.worktree.path_placeholder").to_string())
            }),
            branch: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(branch.clone())
                    .placeholder(rust_i18n::t!("git.worktree.branch_placeholder").to_string())
            }),
            checkout: true,
            error: None,
        });
        let _ = parent;
        cx.notify();
    }

    /// 关闭工作树对话框。
    pub(crate) fn close_git_worktree_modal(&mut self, cx: &mut Context<Self>) {
        self.state.modals.git.worktree = None;
        cx.notify();
    }

    /// 提交创建工作树。
    pub(crate) fn commit_git_worktree(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.worktree.as_ref() else {
            return;
        };
        let path = modal.path.read(cx).value().trim().to_string();
        let branch = modal.branch.read(cx).value().trim().to_string();
        let project = modal.project.clone();
        let start = modal.start.clone();
        let checkout = modal.checkout;
        if path.is_empty() || branch.is_empty() {
            if let Some(modal) = self.state.modals.git.worktree.as_mut() {
                modal.error = Some(rust_i18n::t!("git.worktree.need_fields").to_string());
            }
            cx.notify();
            return;
        }
        self.state.modals.git.worktree = None;
        self.git_create_worktree(&project, path, branch, Some(start), checkout, cx);
        cx.notify();
    }

    /// 对话框主体渲染（接入 dialog_host）。
    pub(crate) fn render_git_worktree_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.state.modals.git.worktree.as_ref() else {
            return div().into_any_element();
        };
        let project = modal.project.clone();
        let start = modal.start.clone();
        let checkout = modal.checkout;
        let error = modal.error.clone();
        let path_state = modal.path.clone();
        let branch_state = modal.branch.clone();
        let root = project.path.display().to_string();

        let body = v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::FolderGit)
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("{root} · {start}")),
                    ),
            )
            .child(crate::widgets::form_field(
                rust_i18n::t!("git.worktree.path_label").to_string(),
                path_state,
                p,
            ))
            .child(crate::widgets::form_field(
                rust_i18n::t!("git.worktree.branch_label").to_string(),
                branch_state,
                p,
            ))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .cursor_pointer()
                    .id("git-worktree-checkout")
                    .child(check(checkout, p))
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("git.worktree.checkout").to_string()),
                    )
                    .on_click({
                        let app = app.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                if let Some(modal) = this.state.modals.git.worktree.as_mut() {
                                    modal.checkout = !modal.checkout;
                                }
                                cx.notify();
                            });
                        }
                    }),
            )
            .when(error.is_some(), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(p.danger)
                        .child(error.unwrap_or_default()),
                )
            });

        let footer = h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(crate::widgets::dialog_cancel_button(
                "git-worktree-cancel",
                rust_i18n::t!("git.project.cancel").to_string(),
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.close_git_worktree_modal(cx);
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "git-worktree-confirm",
                rust_i18n::t!("git.worktree.confirm").to_string(),
                None,
                crate::widgets::DialogConfirmKind::Primary,
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.commit_git_worktree(cx);
                }),
            ));

        crate::widgets::dialog_body(
            "git-worktree-dialog",
            rust_i18n::t!("git.worktree.title").to_string(),
            Some(rust_i18n::t!("git.worktree.hint").to_string()),
            body,
            Some(footer),
        )
    }
}

/// 目录输入框的预填值：只给相对目录名，绝对前缀在提示里说明。
fn branch_dir_text(branch: &str) -> String {
    format!("{branch}-worktree")
}

fn check(checked: bool, p: &Palette) -> AnyElement {
    div()
        .size(px(13.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.0))
        .border_1()
        .border_color(if checked { p.primary } else { p.border })
        .bg(if checked { p.primary } else { p.background })
        .when(checked, |el| {
            el.child(
                Icon::new(IconName::Check)
                    .xsmall()
                    .text_color(p.sidebar_accent_foreground),
            )
        })
        .into_any_element()
}
