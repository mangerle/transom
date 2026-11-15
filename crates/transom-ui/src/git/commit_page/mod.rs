//! 标签页内的提交页：左侧改动树 + 提交消息 + 提交按钮，右侧直接看选中文件的差异。
//!
//! 设计原理（Why）：
//! - 提交不再走模态框：打开仓库先看到「改了什么、能提交什么」，而不是一屏历史；
//!   左侧滚动改动树、右侧常驻差异，提交前核对与写消息在同一视线内完成；
//! - 改动树、勾选、折叠态与差异请求全部复用标签页状态：切到日志再切回来，
//!   勾选、消息草稿、折叠与右侧 diff 都还在；
//! - 消息框与「提交中」状态随标签页存活，事件回来时不需要去找某个弹窗；
//! - 右侧仍是 `diff_view`：与日志页共用同一套 diff 渲染，不维护两套差异视图。
//!
//! 代价与局限：
//! - 左栏固定 440px：窗口很窄时右侧 diff 会先被挤压；后续需要拖拽分隔条再引入。
//! - 提交页与日志页共享 `state.diff`：在日志里看过的提交差异会带到这里，切回来
//!   仍是那份内容，直到用户点新的文件；这样做省掉两份 diff 状态，但不会自动清空。

use super::commit_box::CommitBoxProps;
use super::commit_modal::{GroupDisplay, amend_file_row};
use super::diff_view;
use super::{DIFF_FOLD_ROW_HEIGHT, GitCommitTab, GitWorkspace};
use crate::divider::{DividerEdge, DividerStyled, h_resizable, v_resizable};
use crate::palette::Palette;
use crate::theme::tokens::{size, typography};
use crate::widgets::{underline_tab_active_line, underline_tab_bar_line};
use gpui_kit::base::{Tab, Tabs, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::CommitFile;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 提交页左栏初始宽度：改动树 + 消息够用，实际宽度由可拖拽分隔条控制（范围见 `size_range`）。
const COMMIT_PANEL_WIDTH: f32 = 440.0;
const COMMIT_PANEL_RANGE: (f32, f32) = (320.0, 720.0);

/// 渲染标签页的提交页。
pub(super) fn render_git_commit_page(
    workspace: Entity<GitWorkspace>,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let (
        tab_id,
        display,
        amend,
        submitting,
        message,
        count,
        commit_tab,
        shelf_count,
        stash_count,
        split,
        amend_files,
        amend_files_loading,
        amend_head,
    ) = {
        let ws = workspace.read(cx);
        let state = &ws.state;
        if state.not_repo {
            let error = state
                .log
                .error
                .clone()
                .or_else(|| state.status_error.clone())
                .unwrap_or_else(|| rust_i18n::t!("git.repo.not_repo").to_string());
            return super::workspace::render_git_not_repo_state(
                error,
                state.init_running,
                workspace.clone(),
                p,
            );
        }
        if state.status_loading {
            return super::workspace::hint(rust_i18n::t!("git.overview.loading").to_string(), p);
        }
        if let Some(error) = state.status_error.clone() {
            return super::workspace::hint_colored(error, p.danger);
        }
        let display = GroupDisplay {
            checked: state.checked.clone(),
            collapsed_groups: state.collapsed_groups.clone(),
            by_directory: state.group_by_directory,
            collapsed_dirs: state.collapsed_dirs.clone(),
        };
        let count = state
            .status
            .as_ref()
            .map(|status| status.entries.len())
            .unwrap_or(0);
        (
            ws.tab_id,
            display,
            state.commit_amend,
            state.commit_submitting,
            state.commit_message.clone(),
            count,
            state.commit_tab,
            state.shelves.len(),
            state.stashes.len(),
            state.commit_split.clone(),
            state.amend_files.clone(),
            state.amend_files_loading,
            state
                .log
                .entries
                .first()
                .map(|entry| (entry.short_oid().to_string(), entry.subject.clone())),
        )
    };

    let tabs = render_commit_tabs(&workspace, commit_tab, shelf_count, stash_count, p);
    let content = match commit_tab {
        GitCommitTab::Commit => {
            let checked_count = display.checked.len();
            let toolbar_actions = render_commit_toolbar(&workspace, display.by_directory, p);
            let files = super::commit_modal::change_tree::render_git_group_list(
                workspace.clone(),
                &display,
                p,
                cx,
            );
            let amend_target =
                super::commit_modal::git_amend_target_button(workspace.clone(), p, cx);
            let amend_ws = workspace.clone();
            let amend_toggle = super::commit_modal::git_check_toggle(
                "git-commit-amend",
                rust_i18n::t!("git.commit.amend").to_string(),
                amend,
                p,
                move |_, window, cx| {
                    amend_ws.update(cx, |ws, cx| ws.toggle_commit_amend(window, cx));
                },
            );
            let commit_box = super::commit_box::render_commit_box(
                CommitBoxProps {
                    workspace: workspace.clone(),
                    amend_toggle,
                    amend_target,
                    checked_count,
                    message: &message,
                    submitting,
                },
                p,
                cx,
            );
            let upper_tree = render_upper_change_tree(
                count,
                amend,
                amend_files_loading,
                &amend_files,
                amend_head.as_ref(),
                files,
                p,
            );
            let tree_panel = resizable_panel()
                .size(px(280.0))
                .size_range(px(100.0)..px(800.0))
                .child(upper_tree);
            let box_panel = resizable_panel()
                .size(px(220.0))
                .size_range(px(120.0)..px(600.0))
                .child(commit_box);

            v_flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .child(
                    h_flex()
                        .w_full()
                        .h(rems(size::PANE_HEADER_HEIGHT))
                        .flex_none()
                        .px_2()
                        .gap_2()
                        .items_center()
                        .border_color(p.divider(p.border))
                        .divider_edge(DividerEdge::Bottom, p.divider_width)
                        .child(toolbar_actions),
                )
                .child(
                    div().w_full().flex_1().min_h_0().child(
                        v_resizable(ElementId::Name(
                            format!("git-commit-inner-split-{tab_id}").into(),
                        ))
                        .child(tree_panel)
                        .child(box_panel),
                    ),
                )
                .into_any_element()
        }
        GitCommitTab::Shelf => div()
            .flex_1()
            .min_h_0()
            .child(super::shelf::render_git_shelf_panel(
                workspace.clone(),
                p,
                cx,
            ))
            .into_any_element(),
        GitCommitTab::Stash => div()
            .flex_1()
            .min_h_0()
            .child(super::stash::render_git_stash_panel(
                workspace.clone(),
                p,
                cx,
            ))
            .into_any_element(),
    };

    layout::render_commit_page_split(workspace, tab_id, tabs, content, split, p, cx)
}

fn render_commit_tabs(
    workspace: &Entity<GitWorkspace>,
    commit_tab: GitCommitTab,
    shelf_count: usize,
    stash_count: usize,
    p: &Palette,
) -> AnyElement {
    let shelf_label = if shelf_count > 0 {
        format!("{} ({shelf_count})", rust_i18n::t!("git.shelf.title"))
    } else {
        rust_i18n::t!("git.shelf.title").to_string()
    };
    let stash_label = if stash_count > 0 {
        format!("{} ({stash_count})", rust_i18n::t!("git.stash.title"))
    } else {
        rust_i18n::t!("git.stash.title").to_string()
    };
    let ws_commit = workspace.clone();
    let ws_shelf = workspace.clone();
    let ws_stash = workspace.clone();
    Tabs::new("git-commit-tabs")
        .relative()
        .w_full()
        .h(rems(size::TAB_BAR_HEIGHT))
        .flex_none()
        .flex()
        .px_2()
        .gap_1()
        .items_center()
        .child(underline_tab_bar_line(p))
        .child(commit_tab_btn(
            "git-commit-tab-commit",
            rust_i18n::t!("git.commit.title").to_string(),
            commit_tab == GitCommitTab::Commit,
            p,
            move |_, _, cx| {
                ws_commit.update(cx, |ws, cx| ws.set_commit_tab(GitCommitTab::Commit, cx));
            },
        ))
        .child(commit_tab_btn(
            "git-commit-tab-shelf",
            shelf_label,
            commit_tab == GitCommitTab::Shelf,
            p,
            move |_, _, cx| {
                ws_shelf.update(cx, |ws, cx| ws.set_commit_tab(GitCommitTab::Shelf, cx));
            },
        ))
        .child(commit_tab_btn(
            "git-commit-tab-stash",
            stash_label,
            commit_tab == GitCommitTab::Stash,
            p,
            move |_, _, cx| {
                ws_stash.update(cx, |ws, cx| ws.set_commit_tab(GitCommitTab::Stash, cx));
            },
        ))
        .into_any_element()
}

fn render_commit_toolbar(
    workspace: &Entity<GitWorkspace>,
    by_directory: bool,
    p: &Palette,
) -> impl IntoElement {
    let grouping = super::commit_modal::git_grouping_menu(workspace.clone(), by_directory);
    let ws_refresh = workspace.clone();
    let ws_rollback = workspace.clone();
    let ws_diff = workspace.clone();
    let ws_shelve = workspace.clone();
    let ws_stash = workspace.clone();
    let ws_tree = workspace.clone();
    let ws_expand = workspace.clone();
    let ws_collapse = workspace.clone();
    h_flex()
        .gap_1()
        .items_center()
        .child(
            Button::new("git-commit-refresh")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::REFRESH))
                .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                .on_click(move |_, _window, cx| {
                    ws_refresh.update(cx, |ws, cx| {
                        ws.refresh();
                        cx.notify();
                    });
                }),
        )
        .child(
            Button::new("git-commit-rollback")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::RESET))
                .accessibility_label(rust_i18n::t!("git.menu.discard").to_string())
                .tooltip(rust_i18n::t!("git.menu.discard").to_string())
                .on_click(move |_, _window, cx| {
                    ws_rollback.update(cx, |ws, cx| ws.rollback_checked(cx));
                }),
        )
        .child(
            Button::new("git-commit-show-diff")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::DIFF))
                .accessibility_label(rust_i18n::t!("git.menu.show_diff").to_string())
                .tooltip(rust_i18n::t!("git.menu.show_diff").to_string())
                .on_click(move |_, _window, cx| {
                    ws_diff.update(cx, |ws, cx| ws.open_first_worktree_diff(cx));
                }),
        )
        .child(
            Button::new("git-commit-shelve")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::PATCH))
                .accessibility_label(rust_i18n::t!("git.shelf.create").to_string())
                .tooltip(rust_i18n::t!("git.shelf.create").to_string())
                .on_click(move |_, _window, cx| {
                    ws_shelve.update(cx, |ws, cx| ws.shelve_checked(cx));
                }),
        )
        .child(
            Button::new("git-commit-stash")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::SHELVE))
                .accessibility_label(rust_i18n::t!("git.stash.create").to_string())
                .tooltip(rust_i18n::t!("git.stash.create").to_string())
                .on_click(move |_, _window, cx| {
                    ws_stash.update(cx, |ws, cx| ws.stash_checked(cx));
                }),
        )
        .child(
            Button::new("git-commit-group-tree")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::TREE))
                .when(by_directory, |btn| btn.bg(p.muted.opacity(0.8)))
                .accessibility_label(rust_i18n::t!("git.group.by_directory").to_string())
                .tooltip(rust_i18n::t!("git.group.by_directory").to_string())
                .on_click(move |_, _window, cx| {
                    ws_tree.update(cx, |ws, cx| ws.toggle_group_by_directory(cx));
                }),
        )
        .child(
            Button::new("git-commit-expand-all")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::EXPAND_ALL))
                .accessibility_label(rust_i18n::t!("git.group.expand_all").to_string())
                .tooltip(rust_i18n::t!("git.group.expand_all").to_string())
                .on_click(move |_, _window, cx| {
                    ws_expand.update(cx, |ws, cx| ws.expand_all_groups(cx));
                }),
        )
        .child(
            Button::new("git-commit-collapse-all")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::COLLAPSE_ALL))
                .accessibility_label(rust_i18n::t!("git.group.collapse_all").to_string())
                .tooltip(rust_i18n::t!("git.group.collapse_all").to_string())
                .on_click(move |_, _window, cx| {
                    ws_collapse.update(cx, |ws, cx| ws.collapse_all_groups(cx));
                }),
        )
        .child(grouping)
}

fn render_upper_change_tree(
    count: usize,
    amend: bool,
    amend_files_loading: bool,
    amend_files: &[CommitFile],
    amend_head: Option<&(String, String)>,
    files: AnyElement,
    p: &Palette,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .child(
            h_flex()
                .w_full()
                .h(rems(DIFF_FOLD_ROW_HEIGHT))
                .flex_none()
                .px_2()
                .gap_2()
                .items_center()
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!("git.commit.changes_count", count = count).to_string(),
                    ),
                )
                .child(div().flex_1()),
        )
        // 修正时在上方只读展示目标提交的文件，方便核对这次会追加到哪次提交。
        .when(amend, |el| {
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .bg(p.surface.active)
                    .child(
                        AppIconElement::new(UiIcon::PATCH)
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                amend_head
                                    .map(|(oid, subject)| {
                                        rust_i18n::t!(
                                            "git.commit.amend_group",
                                            oid = oid.as_str(),
                                            subject = subject.as_str()
                                        )
                                        .to_string()
                                    })
                                    .unwrap_or_else(|| {
                                        rust_i18n::t!("git.commit.last_commit").to_string()
                                    }),
                            ),
                    )
                    .child(
                        div().text_xs().text_color(p.muted_foreground).child(
                            rust_i18n::t!("git.commit.file_count", count = amend_files.len())
                                .to_string(),
                        ),
                    ),
            );
            if amend_files_loading {
                group = group.child(super::workspace::hint(
                    rust_i18n::t!("git.overview.loading").to_string(),
                    p,
                ));
            } else if amend_files.is_empty() {
                group = group.child(super::workspace::hint(
                    rust_i18n::t!("git.commit.amend_empty").to_string(),
                    p,
                ));
            } else {
                group = group.children(amend_files.iter().map(|file| amend_file_row(file, p)));
            }
            el.child(group)
        })
        .child(files)
}

mod layout;
use layout::commit_tab_btn;
