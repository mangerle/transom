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
use crate::app::AppShell;
use crate::divider::{h_resizable, v_resizable};
use crate::git::GitCommitTab;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

/// 提交页左栏初始宽度：改动树 + 消息够用，实际宽度由可拖拽分隔条控制（范围见 `size_range`）。
const COMMIT_PANEL_WIDTH: f32 = 440.0;
const COMMIT_PANEL_RANGE: (f32, f32) = (320.0, 720.0);

impl AppShell {
    /// 渲染标签页的提交页。
    pub(crate) fn render_git_commit_page(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 提交成功后消息框的清理要 Window；事件落地时没有，先在这里补一次。
        if self
            .state
            .git_workspace(tab_id)
            .is_some_and(|workspace| workspace.read(cx).state.commit_clear_pending)
        {
            if let Some(workspace) = self.state.git_workspace(tab_id) {
                workspace.update(cx, |workspace, _| {
                    workspace.state.commit_clear_pending = false;
                });
            }
            cx.defer_in(window, move |this, window, cx| {
                if let Some(workspace) = this.state.git_workspace(tab_id) {
                    let commit_message =
                        workspace.update(cx, |workspace, _| workspace.state.commit_message.clone());
                    commit_message.update(cx, |input, cx| input.set_value("", window, cx));
                }
            });
        }

        let (
            project,
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
            let Some(workspace) = self.state.git_workspace(tab_id) else {
                return div().into_any_element();
            };
            let state = &workspace.read(cx).state;
            if state.not_repo {
                let error = state
                    .log
                    .error
                    .clone()
                    .or_else(|| state.status_error.clone())
                    .unwrap_or_else(|| rust_i18n::t!("git.repo.not_repo").to_string());
                let init_running = state.init_running;
                return super::workspace::render_git_not_repo_state(
                    tab_id,
                    error,
                    init_running,
                    cx.entity(),
                    p,
                );
            }
            if state.status_loading {
                return super::workspace::hint(
                    rust_i18n::t!("git.overview.loading").to_string(),
                    p,
                );
            }
            if let Some(error) = state.status_error.clone() {
                return super::workspace::hint_colored(error, p.danger);
            }
            let project = state.project.clone();
            let display = GroupDisplay {
                checked: state.checked.clone(),
                collapsed_groups: state.collapsed_groups.clone(),
                by_directory: state.group_by_directory,
                collapsed_dirs: state.collapsed_dirs.clone(),
            };
            let amend = state.commit_amend;
            let submitting = state.commit_submitting;
            let message = state.commit_message.clone();
            let count = state
                .status
                .as_ref()
                .map(|status| status.entries.len())
                .unwrap_or(0);
            (
                project,
                display,
                amend,
                submitting,
                message,
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
        let tabs = h_flex()
            .w_full()
            .px_2()
            .gap_1()
            .items_center()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(commit_tab_btn(
                "git-commit-tab-commit",
                rust_i18n::t!("git.commit.title").to_string(),
                commit_tab == GitCommitTab::Commit,
                p,
                cx.listener(move |this, _, _, cx| {
                    this.git_set_commit_tab(tab_id, GitCommitTab::Commit, cx);
                }),
            ))
            .child(commit_tab_btn(
                "git-commit-tab-shelf",
                shelf_label,
                commit_tab == GitCommitTab::Shelf,
                p,
                cx.listener(move |this, _, _, cx| {
                    this.git_set_commit_tab(tab_id, GitCommitTab::Shelf, cx);
                }),
            ))
            .child(commit_tab_btn(
                "git-commit-tab-stash",
                stash_label,
                commit_tab == GitCommitTab::Stash,
                p,
                cx.listener(move |this, _, _, cx| {
                    this.git_set_commit_tab(tab_id, GitCommitTab::Stash, cx);
                }),
            ));

        let content = match commit_tab {
            GitCommitTab::Commit => {
                let checked_count = display.checked.len();
                let grouping =
                    self.git_grouping_menu(cx.entity(), project.clone(), display.by_directory);
                let files = self.render_git_group_list(&project, &display, p, cx);
                let amend_target = self.git_amend_target_button(
                    super::commit_modal::GitAmendTargetParams {
                        app: cx.entity(),
                        tab_id,
                        project: &project,
                        palette: p,
                    },
                    cx,
                );

                let branch_chip = self.render_git_branch_chip(project.id, p);
                let refresh_project = project.clone();
                let rollback_project = project.clone();
                let tree_project = project.clone();
                let expand_project = project.clone();
                let collapse_project = project.clone();
                let by_directory = display.by_directory;
                let toolbar_actions = h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("git-commit-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RotateCw)
                            .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                            .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_refresh(&refresh_project, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-rollback")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Undo2)
                            .accessibility_label(rust_i18n::t!("git.menu.discard").to_string())
                            .tooltip(rust_i18n::t!("git.menu.discard").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_rollback_checked(&rollback_project, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-show-diff")
                            .ghost()
                            .xsmall()
                            .icon(IconName::GitCompare)
                            .accessibility_label(rust_i18n::t!("git.menu.show_diff").to_string())
                            .tooltip(rust_i18n::t!("git.menu.show_diff").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_open_first_worktree_diff(tab_id, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-shelve")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PackagePlus)
                            .accessibility_label(rust_i18n::t!("git.shelf.create").to_string())
                            .tooltip(rust_i18n::t!("git.shelf.create").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_shelve_checked(tab_id, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-stash")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Archive)
                            .accessibility_label(rust_i18n::t!("git.stash.create").to_string())
                            .tooltip(rust_i18n::t!("git.stash.create").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_stash_checked(tab_id, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-group-tree")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderTree)
                            .when(by_directory, |btn| btn.bg(p.muted.opacity(0.8)))
                            .accessibility_label(
                                rust_i18n::t!("git.group.by_directory").to_string(),
                            )
                            .tooltip(rust_i18n::t!("git.group.by_directory").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_toggle_group_by_directory(&tree_project, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-expand-all")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronDown)
                            .accessibility_label(rust_i18n::t!("git.group.expand_all").to_string())
                            .tooltip(rust_i18n::t!("git.group.expand_all").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_expand_all_groups(&expand_project, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-collapse-all")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronRight)
                            .accessibility_label(
                                rust_i18n::t!("git.group.collapse_all").to_string(),
                            )
                            .tooltip(rust_i18n::t!("git.group.collapse_all").to_string())
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.git_collapse_all_groups(&collapse_project, cx);
                            })),
                    )
                    .child(grouping);
                let amend_toggle = self.git_check_toggle(
                    "git-commit-amend",
                    rust_i18n::t!("git.commit.amend").to_string(),
                    amend,
                    p,
                    cx.listener(move |this, _, window, cx| {
                        this.git_toggle_commit_amend(tab_id, window, cx);
                    }),
                );

                let commit_box = self.render_commit_box(
                    CommitBoxProps {
                        tab_id,
                        amend_toggle,
                        amend_target,
                        checked_count,
                        message: &message,
                        submitting,
                    },
                    p,
                    cx,
                );

                let upper_tree = v_flex()
                    .w_full()
                    .h_full()
                    .min_h_0()
                    .child(
                        h_flex()
                            .w_full()
                            .px_2()
                            .py_1()
                            .gap_2()
                            .items_center()
                            .child(
                                div().text_xs().text_color(p.muted_foreground).child(
                                    rust_i18n::t!("git.commit.changes_count", count = count)
                                        .to_string(),
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
                                .child(Icon::new(IconName::Package).xsmall().text_color(p.primary))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(
                                            amend_head
                                                .as_ref()
                                                .map(|(oid, subject)| {
                                                    rust_i18n::t!(
                                                        "git.commit.amend_group",
                                                        oid = oid.as_str(),
                                                        subject = subject.as_str()
                                                    )
                                                    .to_string()
                                                })
                                                .unwrap_or_else(|| {
                                                    rust_i18n::t!("git.commit.last_commit")
                                                        .to_string()
                                                }),
                                        ),
                                )
                                .child(
                                    div().text_xs().text_color(p.muted_foreground).child(
                                        rust_i18n::t!(
                                            "git.commit.file_count",
                                            count = amend_files.len()
                                        )
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
                            group = group
                                .children(amend_files.iter().map(|file| amend_file_row(file, p)));
                        }
                        el.child(group)
                    })
                    .child(files);

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
                            .px_2()
                            .py_1()
                            .gap_2()
                            .items_center()
                            .border_color(p.divider(p.border))
                            .border_b_1()
                            .child(branch_chip)
                            .child(div().flex_1())
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
                .child(self.render_git_shelf_panel(tab_id, p, cx))
                .into_any_element(),
            GitCommitTab::Stash => div()
                .flex_1()
                .min_h_0()
                .child(self.render_git_stash_panel(tab_id, p, cx))
                .into_any_element(),
        };

        let left = v_flex()
            .w_full()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .child(tabs)
            .child(content);

        // 右栏每次重新借一次状态：左栏构建期间要 `&mut self`。
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return left.into_any_element();
        };
        let right = {
            let state = &workspace.read(cx).state;
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(diff_view::render(state, cx.entity(), tab_id, p))
        };

        let left_panel = resizable_panel()
            .size(px(COMMIT_PANEL_WIDTH))
            .size_range(px(COMMIT_PANEL_RANGE.0)..px(COMMIT_PANEL_RANGE.1))
            .child(left);
        let right_panel = resizable_panel().child(right);
        div()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(
                h_resizable(ElementId::Name(format!("git-commit-split-{tab_id}").into()))
                    .with_state(&split)
                    .child(left_panel)
                    .child(right_panel),
            )
            .into_any_element()
    }
}

/// 提交页顶部单个页签胶囊按钮。
fn commit_tab_btn(
    id: &'static str,
    label: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .py_1()
        .text_sm()
        .cursor_pointer()
        .font_weight(if active {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        })
        .border_b_2()
        .border_color(if active { p.primary } else { p.background })
        .child(label)
        .on_click(on_click)
}
