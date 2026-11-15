//! Git 推送模态弹窗主渲染逻辑（双栏布局、目标分支标头与底栏安全选项）。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::PushTagsMode;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::views::{render_commit_detail_card, render_commit_row, render_file_row};
use super::{GitPushModalEvent, GitPushModalView, is_git_branch_protected};

/// 渲染推送模态弹窗主体。
pub(super) fn render_push_dialog(
    modal: &GitPushModalView,
    _window: &mut Window,
    cx: &mut Context<GitPushModalView>,
    p: &Palette,
) -> AnyElement {
    let is_protected = is_git_branch_protected(&modal.remote_branch, cx);

    let mut container = v_flex()
        .w_full()
        .child(render_push_target_bar(modal, is_protected, p));
    if let Some(err) = modal.error.as_deref() {
        container = container.child(render_push_error_banner(err, p));
    }
    container
        .child(render_push_split_body(modal, cx, p))
        .child(render_push_footer(modal, is_protected, cx, p))
        .into_any_element()
}

/// 渲染顶部目标路径胶囊与提交计数栏。
fn render_push_target_bar(modal: &GitPushModalView, is_protected: bool, p: &Palette) -> AnyElement {
    let commits_count = modal.commits.len();
    h_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .items_center()
        .bg(p.surface.pressed)
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .px_2()
                .py_1()
                .bg(p.surface.active)
                .rounded_sm()
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::VCS))
                        .xsmall()
                        .text_color(p.primary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.foreground)
                        .child(modal.project.display_name()),
                ),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::VCS))
                        .xsmall()
                        .text_color(p.primary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(modal.branch.clone()),
                ),
        )
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::RIGHT))
                .xsmall()
                .text_color(p.muted_foreground),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::LANGUAGE))
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format!("{} : {}", modal.remote, modal.remote_branch)),
                ),
        )
        .when(is_protected, |this| {
            this.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .bg(p.warning.opacity(0.15))
                    .border_1()
                    .border_color(p.warning.opacity(0.3))
                    .child(
                        AppIconElement::new(AppIcon::Ui(UiIcon::LOCKED))
                            .xsmall()
                            .text_color(p.warning),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.warning)
                            .child(rust_i18n::t!("settings.git_protected_branches").to_string()),
                    ),
            )
        })
        .child(div().flex_1())
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.push.commits_count", count = commits_count).to_string()),
        )
        .into_any_element()
}

/// 渲染双栏主体（左栏待推送提交列表，右栏提交详情与变更文件）。
fn render_push_split_body(
    modal: &GitPushModalView,
    cx: &mut Context<GitPushModalView>,
    p: &Palette,
) -> AnyElement {
    let commits_list: Vec<AnyElement> = if modal.commits_loading {
        vec![render_muted_placeholder(
            rust_i18n::t!("git.overview.loading").to_string(),
            p,
        )]
    } else if modal.commits.is_empty() {
        vec![render_muted_placeholder(
            rust_i18n::t!("git.push.empty_commits").to_string(),
            p,
        )]
    } else {
        modal
            .commits
            .iter()
            .map(|commit| {
                let is_selected = modal.selected_oid.as_deref() == Some(&commit.oid);
                render_commit_row(commit, is_selected, cx, p)
            })
            .collect()
    };

    let commit_card: Option<AnyElement> = modal
        .selected_oid
        .as_ref()
        .and_then(|oid| modal.commits.iter().find(|c| &c.oid == oid))
        .map(|commit| render_commit_detail_card(commit, cx, p));

    let files_list: Vec<AnyElement> = if modal.files_loading {
        vec![render_muted_placeholder(
            rust_i18n::t!("git.overview.loading").to_string(),
            p,
        )]
    } else if modal.commit_files.is_empty() {
        vec![render_muted_placeholder(
            rust_i18n::t!("git.push.no_files").to_string(),
            p,
        )]
    } else {
        modal
            .commit_files
            .iter()
            .map(|file| render_file_row(file, modal.tab_id, cx, p))
            .collect()
    };

    h_flex()
        .w_full()
        .h(px(340.0))
        .min_h_0()
        .child(
            v_flex()
                .w(px(380.0))
                .h_full()
                .min_h_0()
                .border_r_1()
                .border_color(p.divider(p.border))
                .child(render_pane_header(
                    rust_i18n::t!("git.push.commits").to_string(),
                    p,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .children(commits_list),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .h_full()
                .min_h_0()
                .children(commit_card)
                .child(render_pane_header(
                    rust_i18n::t!("git.commit.file_count", count = modal.commit_files.len())
                        .to_string(),
                    p,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .children(files_list),
                ),
        )
        .into_any_element()
}

fn render_muted_placeholder(text: String, p: &Palette) -> AnyElement {
    div()
        .p_4()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}

fn render_pane_header(title: String, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .bg(p.muted.opacity(0.25))
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted_foreground)
                .child(title),
        )
        .into_any_element()
}

/// 渲染推送错误横幅。
fn render_push_error_banner(err: &str, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .px_3()
        .py_1p5()
        .gap_2()
        .items_center()
        .bg(p.tone.danger_subtle)
        .border_b_1()
        .border_color(p.danger.opacity(0.4))
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::ERROR))
                .xsmall()
                .text_color(p.danger),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(p.danger)
                .child(err.to_string()),
        )
        .into_any_element()
}

/// 渲染底部选项与动作栏。
fn render_push_footer(
    modal: &GitPushModalView,
    is_protected: bool,
    cx: &mut Context<GitPushModalView>,
    p: &Palette,
) -> AnyElement {
    let running = modal.running;
    let force = modal.force;
    let tags_mode = modal.tags_mode;

    let tags_label = match tags_mode {
        PushTagsMode::None => rust_i18n::t!("git.push.push_tags").to_string(),
        PushTagsMode::FollowTags => format!(
            "{} ({})",
            rust_i18n::t!("git.push.push_tags"),
            rust_i18n::t!("git.push.follow_tags")
        ),
        PushTagsMode::AllTags => format!(
            "{} ({})",
            rust_i18n::t!("git.push.push_tags"),
            rust_i18n::t!("git.push.all_tags")
        ),
    };

    h_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .id("push-tags-toggle")
                .gap_1()
                .items_center()
                .cursor_pointer()
                .child(crate::git::commit_modal::check_box(
                    tags_mode != PushTagsMode::None,
                    p,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(tags_label),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cycle_tags_mode(cx);
                })),
        )
        .child(
            h_flex()
                .id("push-force-toggle")
                .gap_1()
                .items_center()
                .cursor_pointer()
                .child(crate::git::commit_modal::check_box(force, p))
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_protected || !force {
                            p.muted_foreground
                        } else {
                            p.danger
                        })
                        .child(if is_protected {
                            format!(
                                "{} ({})",
                                rust_i18n::t!("git.push.force"),
                                rust_i18n::t!("git.push.protected_branch_warning")
                            )
                        } else {
                            format!("{} (--force-with-lease)", rust_i18n::t!("git.push.force"))
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !is_protected {
                        this.toggle_force(cx);
                    }
                })),
        )
        .child(
            h_flex()
                .id("push-no-verify-toggle")
                .gap_1()
                .items_center()
                .cursor_pointer()
                .child(crate::git::commit_modal::check_box(modal.no_verify, p))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("git.push.no_verify").to_string()),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_no_verify(cx);
                })),
        )
        .child(div().flex_1())
        .child(
            Button::new("git-push-cancel")
                .ghost()
                .small()
                .label(rust_i18n::t!("git.project.cancel").to_string())
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(GitPushModalEvent::Close);
                })),
        )
        .child(
            Button::new("git-push-submit")
                .primary()
                .small()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::UPLOAD)).small())
                .disabled(running)
                .label(if running {
                    rust_i18n::t!("git.overview.loading").to_string()
                } else if force {
                    rust_i18n::t!("git.push.force_push").to_string()
                } else {
                    rust_i18n::t!("git.menu.push").to_string()
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.submit(cx);
                })),
        )
        .into_any_element()
}
