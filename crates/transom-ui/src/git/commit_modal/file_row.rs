//! 改动树里的单行文件：勾选、图标、语义着色文件名、灰色目录后缀与右键菜单。
//!
//! 设计原理（Why）：文件行通过文件名颜色区分变更语义（如新增绿、修改蓝、
//! 删除灰、未跟踪/冲突红），因此行尾不再重复堆叠状态字母与差异图标按钮；
//! 单击与双击统一在右侧差异区打开比对，不再弹出独立差异弹窗。

use super::{ChangeGroup, check_box, check_placeholder, split_path};
use crate::app::AppShell;
use crate::git::actions::GitDiffRequest;
use crate::git::{change_accent_color, primary_record, untracked_visual};
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{GitAction, GitProject, GitSide, StatusEntry};
use transom_ui_icons::UiIcon;
use transom_ui_icons::{AppIcon, AppIconElement};

/// 某个改动行「看差异」该跟谁比：已暂存看 HEAD→索引，其余看索引→工作区。
pub(crate) fn worktree_diff_request(group: ChangeGroup, path: String) -> GitDiffRequest {
    let (old, new, left_label, right_label) = match group {
        ChangeGroup::Staged => (
            GitSide::Commit("HEAD".into()),
            GitSide::Index,
            rust_i18n::t!("git.diff.head").to_string(),
            rust_i18n::t!("git.diff.index").to_string(),
        ),
        _ => (
            GitSide::Index,
            GitSide::Worktree,
            rust_i18n::t!("git.diff.index").to_string(),
            rust_i18n::t!("git.diff.worktree").to_string(),
        ),
    };
    GitDiffRequest {
        path,
        old,
        new,
        left_label,
        right_label,
    }
}

/// 文件行的展示参数：项目、条目、分组、勾选集合与缩进一次传齐，
/// 平铺入参超过 7 个，收成参数对象。
pub(crate) struct GitFileRowProps<'a> {
    pub(crate) project: &'a GitProject,
    pub(crate) entry: &'a StatusEntry,
    pub(crate) group: ChangeGroup,
    pub(crate) checked: &'a HashSet<String>,
    pub(crate) indent: f32,
    pub(crate) selected: bool,
}

impl AppShell {
    /// 一行文件：勾选 + 图标 + 语义着色文件名 + 灰色目录后缀。
    pub(crate) fn git_file_row(
        &mut self,
        props: &GitFileRowProps<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project = props.project;
        let entry = props.entry;
        let group = props.group;
        let checked = props.checked;
        let indent = props.indent;
        let selected = props.selected;
        let path = entry.path.clone();
        let visual = if group == ChangeGroup::Untracked {
            untracked_visual()
        } else {
            primary_record(&entry.record)
        };
        let is_checked = checked.contains(&path);
        let element_id = format!("git-row-{}-{path}", group.key());
        let check_id = format!("{element_id}-check");
        let old_path = entry.old_path.clone();
        let app: Entity<AppShell> = cx.entity().clone();

        let check_project = project.clone();
        let check_path = path.clone();
        let menu_path = path.clone();
        let (file_name, dir_prefix) = split_path(&path);
        let tooltip_path = path.clone();

        let mut row = h_flex()
            .id(ElementId::Name(element_id.into()))
            .h(px(22.0))
            .pl(px(indent))
            .pr_2()
            .gap_2()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|style| style.bg(p.list_hover))
            .tooltip(move |window, cx| Tooltip::new(tooltip_path.clone()).build(window, cx));
        if selected {
            // 当前右侧正在比对的文件：常驻激活底色，与悬停区分。
            row = row.bg(p.list_active);
        }
        // 忽略组的文件行只读：没有勾选框、差异按钮和右键动作，名称置灰。
        if group.actionable() {
            row = row.child(
                div()
                    .id(ElementId::Name(check_id.into()))
                    .flex_none()
                    .child(check_box(is_checked, p))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        this.toggle_git_checked(&check_project, &check_path, cx);
                    })),
            );
        } else {
            row = row.child(check_placeholder());
        }
        row = row
            .child(
                AppIconElement::new(AppIcon::for_file(&path))
                    .with_rems(rems(crate::theme::tokens::control::ICON)),
            )
            .child(
                div()
                    .text_sm()
                    .flex_none()
                    .max_w(px(220.0))
                    .overflow_hidden()
                    .truncate()
                    .text_color(if group.actionable() {
                        change_accent_color(visual.accent, p)
                    } else {
                        p.muted_foreground
                    })
                    .child(file_name),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(dir_prefix),
            )
            .child(
                old_path
                    .map(|old| {
                        div()
                            .max_w(px(140.0))
                            .overflow_hidden()
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(old)
                            .into_any_element()
                    })
                    .unwrap_or_else(|| div().into_any_element()),
            );
        if group.actionable() {
            let click_project = project.clone();
            let click_path = path.clone();
            row = row.on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                // 双击的第一次点击已经完成同一动作；忽略后续点击，避免重复发差异请求。
                if event.click_count() > 1 {
                    return;
                }
                this.git_open_worktree_diff(&click_project, group, click_path.clone(), cx);
            }));
        }
        if group.actionable() {
            let menu_project = project.clone();
            row.context_menu(move |menu, _window, _cx| {
                render_file_menu(menu, app.clone(), &menu_project, group, menu_path.clone())
            })
            .into_any_element()
        } else {
            row.into_any_element()
        }
    }
}

/// 文件行右键菜单。
fn render_file_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    project: &GitProject,
    group: ChangeGroup,
    path: String,
) -> PopupMenu {
    let mut menu = menu;
    if group != ChangeGroup::Staged {
        let app_stage = app.clone();
        let stage_project = project.clone();
        let stage_path = path.clone();
        menu = menu.item(menu_item(
            rust_i18n::t!("git.menu.stage").to_string(),
            AppIcon::Ui(UiIcon::ADD),
            move |_, _, cx| {
                run_paths(
                    &app_stage,
                    &stage_project,
                    GitAction::Stage,
                    &stage_path,
                    cx,
                );
            },
        ));
    } else {
        let app_unstage = app.clone();
        let unstage_project = project.clone();
        let unstage_path = path.clone();
        menu = menu.item(menu_item(
            rust_i18n::t!("git.menu.unstage").to_string(),
            AppIcon::Ui(UiIcon::REMOVE),
            move |_, _, cx| {
                run_paths(
                    &app_unstage,
                    &unstage_project,
                    GitAction::Unstage,
                    &unstage_path,
                    cx,
                );
            },
        ));
    }
    let app_discard = app.clone();
    let discard_project = project.clone();
    let discard_path = path.clone();
    let discard_label = if group == ChangeGroup::Untracked {
        rust_i18n::t!("git.menu.delete").to_string()
    } else {
        rust_i18n::t!("git.menu.discard").to_string()
    };
    let diff_app = app.clone();
    let diff_project = project.clone();
    let diff_path = path.clone();
    let copy_app = app.clone();
    let copy_path = path.clone();
    let refresh_app = app.clone();
    let refresh_project = project.clone();
    let conflict_app = app.clone();
    let conflict_project = project.clone();
    let mut menu = menu.item(menu_item(
        rust_i18n::t!("git.conflict.resolve_menu").to_string(),
        AppIcon::Ui(UiIcon::MERGE),
        move |_, _, cx| {
            let p = conflict_project.clone();
            conflict_app.update(cx, |this, cx| {
                this.open_git_conflict_modal(&p, cx);
            });
        },
    ));
    menu = menu.item(menu_item(
        rust_i18n::t!("git.menu.show_diff").to_string(),
        AppIcon::Ui(UiIcon::DIFF),
        move |_, _, cx| {
            let project = diff_project.clone();
            let path = diff_path.clone();
            diff_app.update(cx, |this, cx| {
                this.git_open_worktree_diff(&project, group, path, cx);
            });
        },
    ));
    menu = menu.item(menu_item(
        discard_label,
        AppIcon::Ui(UiIcon::UNDO),
        move |_, _, cx| {
            let action = if group == ChangeGroup::Untracked {
                GitAction::DeleteUntracked
            } else {
                GitAction::Discard
            };
            run_paths(&app_discard, &discard_project, action, &discard_path, cx);
        },
    ));
    menu = menu.item(menu_item(
        rust_i18n::t!("git.menu.copy_path").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        move |_, _, cx| {
            copy_app.update(cx, |this, cx| {
                this.write_clipboard(&copy_path, cx);
            });
        },
    ));
    menu.item(menu_item(
        rust_i18n::t!("git.menu.refresh").to_string(),
        AppIcon::Ui(UiIcon::REFRESH),
        move |_, _, cx| {
            let project = refresh_project.clone();
            refresh_app.update(cx, |this, cx| {
                this.git_refresh(&project, cx);
            });
        },
    ))
}

/// 菜单动作统一走「按路径批量执行」（直接操作确定的项目仓库）。
fn run_paths(
    app: &Entity<AppShell>,
    project: &GitProject,
    action: GitAction,
    path: &str,
    cx: &mut App,
) {
    let path = path.to_string();
    let project = project.clone();
    app.update(cx, |this, cx| {
        this.git_run_paths(&project, action, vec![path], cx);
    });
}

/// 修正目标提交里的一行只读文件：与改动树同一套状态色，没有勾选框与动作。
pub(crate) fn amend_file_row(file: &transom_core::CommitFile, p: &Palette) -> AnyElement {
    let visual = crate::git::change_visual(file.change);
    let accent = change_accent_color(visual.accent, p);
    let (name, dir) = split_path(&file.path);
    let tooltip_path = file.path.clone();
    h_flex()
        .id(ElementId::Name(
            format!("git-amend-file-{}", file.path).into(),
        ))
        .h(px(22.0))
        .pl(px(24.0))
        .pr_2()
        .gap_2()
        .items_center()
        .overflow_hidden()
        .tooltip(move |window, cx| Tooltip::new(tooltip_path.clone()).build(window, cx))
        .child(div().w(px(14.0)))
        .child(
            AppIconElement::new(AppIcon::for_file(&file.path))
                .with_rems(rems(crate::theme::tokens::control::ICON)),
        )
        .child(
            div()
                .text_sm()
                .flex_none()
                .max_w(px(220.0))
                .overflow_hidden()
                .truncate()
                .text_color(accent)
                .child(name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .truncate()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(dir),
        )
        .into_any_element()
}
