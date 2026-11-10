//! Git 推送模态弹窗主渲染逻辑（双栏布局、目标分支标头与底栏安全选项）。
//!
//! 设计原理（Why）：
//! - 全面贯彻 openidea / IntelliJ IDEA Git 推送模态交互标准；
//! - 顶部明确标注本地到远端的分支映射；
//! - 双栏左侧展示全部待推送提交历史（Outgoing Commits），右侧展示选中提交的作者详情与文件变更；
//! - 底部包含标签策略（--follow-tags / --tags）、受保护分支防护的强制推送与跳过钩子选项。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::PushTagsMode;

use super::views::{render_commit_detail_card, render_commit_row, render_file_row};

/// 渲染推送模态弹窗主体。
pub(super) fn render_push_dialog(
    shell: &AppShell,
    app: &Entity<AppShell>,
    _window: &mut Window,
    _cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(modal) = shell.state.modals.git.push.as_ref() else {
        return div().into_any_element();
    };

    let branch = &modal.branch;
    let remote = &modal.remote;
    let remote_branch = &modal.remote_branch;
    let running = modal.running;
    let tags_mode = modal.tags_mode;
    let force = modal.force;
    let no_verify = modal.no_verify;
    let commits_count = modal.commits.len();
    let files_count = modal.commit_files.len();
    let is_protected = shell.is_git_branch_protected(remote_branch, _cx);
    let tab_id = shell.git_tab_for_project(modal.project.id, _cx);

    // 1. 顶部目标路径胶囊与统计
    let target_bar = h_flex()
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
                    Icon::new(IconName::FolderGit)
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
                    Icon::new(IconName::GitBranch)
                        .xsmall()
                        .text_color(p.primary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(branch.clone()),
                ),
        )
        .child(
            Icon::new(IconName::ArrowRight)
                .xsmall()
                .text_color(p.muted_foreground),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    Icon::new(IconName::Globe)
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format!("{remote} : {remote_branch}")),
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
                    .child(Icon::new(IconName::Shield).xsmall().text_color(p.warning))
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
        );

    // 2. 左栏：待推送提交历史（Outgoing Commits）
    let commits_list: Vec<AnyElement> = if modal.commits_loading {
        vec![
            div()
                .p_4()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.overview.loading").to_string())
                .into_any_element(),
        ]
    } else if modal.commits.is_empty() {
        vec![
            div()
                .p_4()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.push.empty_commits").to_string())
                .into_any_element(),
        ]
    } else {
        modal
            .commits
            .iter()
            .map(|commit| {
                let is_selected = modal.selected_oid.as_deref() == Some(&commit.oid);
                render_commit_row(commit, is_selected, app, p)
            })
            .collect()
    };

    // 3. 右栏：选中提交详情与改动文件列表
    let selected_commit = modal
        .selected_oid
        .as_ref()
        .and_then(|oid| modal.commits.iter().find(|c| &c.oid == oid));

    let commit_card: Option<AnyElement> =
        selected_commit.map(|commit| render_commit_detail_card(commit, app, p));

    let files_list: Vec<AnyElement> = if modal.files_loading {
        vec![
            div()
                .p_4()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.overview.loading").to_string())
                .into_any_element(),
        ]
    } else if modal.commit_files.is_empty() {
        vec![
            div()
                .p_4()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.push.no_files").to_string())
                .into_any_element(),
        ]
    } else {
        modal
            .commit_files
            .iter()
            .map(|file| render_file_row(file, tab_id, app, p))
            .collect()
    };

    // 4. 双栏容器
    let split_body = h_flex()
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
                .child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .py_1p5()
                        .bg(p.muted.opacity(0.25))
                        .border_b_1()
                        .border_color(p.divider(p.border))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("git.push.commits").to_string()),
                        ),
                )
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
                .child(
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
                                .child(
                                    rust_i18n::t!("git.commit.file_count", count = files_count)
                                        .to_string(),
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .children(files_list),
                ),
        );

    // 5. 错误横幅
    let error_banner = modal.error.as_ref().map(|err| {
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
                Icon::new(IconName::CircleAlert)
                    .xsmall()
                    .text_color(p.danger),
            )
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(p.danger)
                    .child(err.clone()),
            )
    });

    // 6. 底部选项与动作栏
    let app_tags = app.clone();
    let app_force = app.clone();
    let app_no_verify = app.clone();
    let app_cancel = app.clone();
    let app_submit = app.clone();

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

    let footer = h_flex()
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
                .on_click(move |_, _, cx| {
                    app_tags.update(cx, |this, cx| this.git_cycle_push_tags_mode(cx));
                }),
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
                        .text_color(if is_protected {
                            p.muted_foreground
                        } else if force {
                            p.danger
                        } else {
                            p.muted_foreground
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
                .on_click(move |_, _, cx| {
                    if !is_protected {
                        app_force.update(cx, |this, cx| this.git_toggle_push_force(cx));
                    }
                }),
        )
        .child(
            h_flex()
                .id("push-no-verify-toggle")
                .gap_1()
                .items_center()
                .cursor_pointer()
                .child(crate::git::commit_modal::check_box(no_verify, p))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("git.push.no_verify").to_string()),
                )
                .on_click(move |_, _, cx| {
                    app_no_verify.update(cx, |this, cx| this.git_toggle_push_no_verify(cx));
                }),
        )
        .child(div().flex_1())
        .child(
            Button::new("git-push-cancel")
                .ghost()
                .small()
                .label(rust_i18n::t!("git.project.cancel").to_string())
                .on_click(move |_, _, cx| {
                    app_cancel.update(cx, |this, cx| this.close_git_push_modal(cx));
                }),
        )
        .child(
            Button::new("git-push-submit")
                .primary()
                .small()
                .icon(IconName::ArrowUpFromLine)
                .disabled(running)
                .label(if running {
                    rust_i18n::t!("git.overview.loading").to_string()
                } else if force {
                    rust_i18n::t!("git.push.force_push").to_string()
                } else {
                    rust_i18n::t!("git.menu.push").to_string()
                })
                .on_click(move |_, _, cx| {
                    app_submit.update(cx, |this, cx| this.submit_git_push_modal(cx));
                }),
        );

    let mut container = v_flex().w_full().child(target_bar);
    if let Some(banner) = error_banner {
        container = container.child(banner);
    }
    container.child(split_body).child(footer).into_any_element()
}
