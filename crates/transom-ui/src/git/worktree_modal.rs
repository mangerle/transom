//! 「新建工作树」对话框。
//!
//! 设计原理（Why）：
//! - 「从这里新建工作树」是一次带三个输入项的确认：目录、分支名、起点；
//! - 起点由调用方带进来（哪个分支/标签/提交），对话框里只读展示；
//! - 目录默认值取相对目录名，具体绝对前缀在提示里说明；
//! - 本模块只维护实体自身输入态与 UI 渲染，提交或关闭通过
//!   [`GitWorktreeModalEvent`] 上报外壳桥接层。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::GitProject;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 新建工作树对话框向上层发送的事件。
pub(crate) enum GitWorktreeModalEvent {
    Submit {
        project: GitProject,
        path: String,
        branch: String,
        start: String,
        checkout: bool,
    },
    Close,
}

/// 新建工作树的表单实体视图。
pub(crate) struct GitWorktreeModalView {
    pub project: GitProject,
    /// 起点引用（分支名 / 标签名 / 提交号）。
    pub start: String,
    pub path: Entity<InputState>,
    pub branch: Entity<InputState>,
    /// 创建后是否同时检出该分支。
    pub checkout: bool,
    pub error: Option<String>,
}

impl EventEmitter<GitWorktreeModalEvent> for GitWorktreeModalView {}

impl Render for GitWorktreeModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl GitWorktreeModalView {
    pub(crate) fn new(
        project: GitProject,
        start: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let branch_name = start.replace('/', "-");
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(branch_dir_text(&branch_name))
                .placeholder(rust_i18n::t!("git.worktree.path_placeholder").to_string())
        });
        let branch = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(branch_name)
                .placeholder(rust_i18n::t!("git.worktree.branch_placeholder").to_string())
        });
        Self {
            project,
            start: start.to_string(),
            path,
            branch,
            checkout: true,
            error: None,
        }
    }

    /// 提交创建工作树：校验必填字段，通过后发送 Submit 事件。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        let path = self.path.read(cx).value().trim().to_string();
        let branch = self.branch.read(cx).value().trim().to_string();
        if path.is_empty() || branch.is_empty() {
            self.error = Some(rust_i18n::t!("git.worktree.need_fields").to_string());
            cx.notify();
            return;
        }
        self.error = None;
        cx.emit(GitWorktreeModalEvent::Submit {
            project: self.project.clone(),
            path,
            branch,
            start: self.start.clone(),
            checkout: self.checkout,
        });
    }

    fn render_body(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let root = self.project.path.display().to_string();
        let start = self.start.clone();
        let checkout = self.checkout;
        let error = self.error.clone();
        let path_state = self.path.clone();
        let branch_state = self.branch.clone();

        let body = v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER))
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.checkout = !this.checkout;
                        cx.notify();
                    })),
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
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(GitWorktreeModalEvent::Close);
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "git-worktree-confirm",
                rust_i18n::t!("git.worktree.confirm").to_string(),
                None,
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.submit(cx);
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
                AppIconElement::new(AppIcon::Ui(UiIcon::CHECK))
                    .xsmall()
                    .text_color(p.sidebar_accent_foreground),
            )
        })
        .into_any_element()
}
