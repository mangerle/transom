//! Git 推送模态弹窗渲染实现（双栏与底部操作）。
//!
//! 设计原理（Why）：
//! - 将推送弹窗视图构建逻辑与状态管理分离，保持单个源码文件在 500 行以内；
//! - 左栏展示待推送到远端的本地提交历史（Outgoing Commits），右栏展示选中提交包含的文件清单；
//! - 底部集成「推送标签」与「强制推送」安全选项，避免误推或无意覆盖远端分支。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::*;
use transom_core::ChangeCode;

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
    let push_tags = modal.push_tags;
    let force = modal.force;
    let commits_count = modal.commits.len();
    let files_count = modal.commit_files.len();

    // 顶部目标路径胶囊
    let target_bar = h_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .items_center()
        .bg(p.muted.opacity(0.4))
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            Icon::new(IconName::GitBranch)
                .xsmall()
                .text_color(p.primary),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(p.foreground)
                .child(branch.clone()),
        )
        .child(
            Icon::new(IconName::ArrowRight)
                .xsmall()
                .text_color(p.muted_foreground),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(format!("{remote} : {remote_branch}")),
        )
        .child(div().flex_1())
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("git.push.commits_count", count = commits_count).to_string()),
        );

    // 左栏：待推送提交列表
    let commits_list: Vec<AnyElement> = if modal.commits.is_empty() {
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
                let oid_clone = commit.oid.clone();
                let app_select = app.clone();
                let item_id = ElementId::Name(format!("push-commit-{}", commit.oid).into());
                h_flex()
                    .id(item_id)
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .bg(if is_selected {
                        p.primary.opacity(0.18)
                    } else {
                        p.background
                    })
                    .hover(|style| {
                        if !is_selected {
                            style.bg(p.muted.opacity(0.4))
                        } else {
                            style
                        }
                    })
                    .child(
                        Icon::new(IconName::GitCommitHorizontal)
                            .xsmall()
                            .text_color(if is_selected {
                                p.primary
                            } else {
                                p.muted_foreground
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(if is_selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(p.foreground)
                                    .truncate()
                                    .child(commit.subject.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(div().child(commit.author_name.clone()))
                                    .child(div().child(commit.short_oid().to_string())),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        let oid = oid_clone.clone();
                        app_select.update(cx, |this, cx| {
                            this.git_select_push_commit(oid, cx);
                        });
                    })
                    .into_any_element()
            })
            .collect()
    };

    // 右栏：改动文件清单
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
            .map(|file| {
                let change = file.change;
                let (color, label) = match change {
                    ChangeCode::Added => (p.success, "A"),
                    ChangeCode::Deleted => (p.danger, "D"),
                    ChangeCode::Renamed => (p.primary, "R"),
                    ChangeCode::Copied => (p.primary, "C"),
                    ChangeCode::TypeChange => (p.warning, "T"),
                    _ => (p.warning, "M"),
                };
                h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .hover(|s| s.bg(p.muted.opacity(0.3)))
                    .child(
                        div()
                            .w(px(14.0))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(color)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(p.foreground)
                            .child(file.path.clone()),
                    )
                    .into_any_element()
            })
            .collect()
    };

    // 双栏容器
    let split_body = h_flex()
        .w_full()
        .h(px(320.0))
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
                        .py_1()
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

    // 底部动作栏：标签、强制推送与推送/取消按钮
    let app_tags = app.clone();
    let app_force = app.clone();
    let app_cancel = app.clone();
    let app_submit = app.clone();

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
                .child(crate::git::commit_modal::check_box(push_tags, p))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("git.push.push_tags").to_string()),
                )
                .on_click(move |_, _, cx| {
                    app_tags.update(cx, |this, cx| this.git_toggle_push_tags(cx));
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
                        .text_color(if force { p.danger } else { p.muted_foreground })
                        .child(rust_i18n::t!("git.push.force").to_string()),
                )
                .on_click(move |_, _, cx| {
                    app_force.update(cx, |this, cx| this.git_toggle_push_force(cx));
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
                .label(if force {
                    rust_i18n::t!("git.push.force_push").to_string()
                } else {
                    rust_i18n::t!("git.menu.push").to_string()
                })
                .on_click(move |_, _, cx| {
                    app_submit.update(cx, |this, cx| this.submit_git_push_modal(cx));
                }),
        );

    // 错误提示条
    let error_banner = modal.error.as_ref().map(|err| {
        h_flex()
            .w_full()
            .px_3()
            .py_1p5()
            .gap_2()
            .items_center()
            .bg(p.danger.opacity(0.12))
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

    let mut container = v_flex().w_full().child(target_bar);
    if let Some(banner) = error_banner {
        container = container.child(banner);
    }
    container.child(split_body).child(footer).into_any_element()
}
