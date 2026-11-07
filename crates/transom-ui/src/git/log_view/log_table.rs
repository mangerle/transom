//! 日志提交表：表头、列开关、行渲染与泳道自绘入口。
//!
//! 设计原理（Why）：提交表是日志工作区里唯一逐行渲染的热路径，把它与右键菜单、
//! 引用栏分开成模块，改列宽/列开关时只碰这一处；行渲染需要的「本行图形态」
//! 收进 [`RowShape`]，避免七个平铺入参越过规范上限。

use super::log_refs::ref_chip;
use super::menus::{CommitMenuTarget, render_commit_menu};
use super::{placeholder, placeholder_colored};
use crate::app::AppShell;
use crate::git::graph_paint::{self, GraphCell};
use crate::git::relative_time;
use crate::git::types::{LogColumn, LogColumns};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitProject, TabId};

type State = crate::git::GitWorkspaceState;

/// 单条提交行高与泳道格宽（px）。
pub(crate) const ROW_HEIGHT: f32 = graph_paint::ROW_HEIGHT;

/// 日志列宽：作者与时间。
const AUTHOR_WIDTH: f32 = 130.0;

const TIME_WIDTH: f32 = 96.0;

/// 中栏提交表。
pub(crate) fn render_log_table(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> AnyElement {
    let lane_count = state
        .log
        .rows
        .iter()
        .map(|row| row.lane + 1 + row.parents.len())
        .max()
        .unwrap_or(1)
        .clamp(1, 8);
    let selected = state.selected_commit;
    let mut rows: Vec<AnyElement> = Vec::with_capacity(state.log.entries.len() + 2);
    if state.log.loading && state.log.entries.is_empty() {
        rows.push(placeholder(rust_i18n::t!("git.log.loading").to_string(), p));
    } else if let Some(error) = state.log.error.clone() {
        rows.push(placeholder_colored(error, p.danger));
    } else if state.log.entries.is_empty() {
        rows.push(placeholder(rust_i18n::t!("git.log.empty").to_string(), p));
    }
    let cur_branch = state.status.as_ref().and_then(|status| match &status.head {
        transom_core::HeadState::OnBranch(name) => Some(name.clone()),
        _ => None,
    });
    for (index, entry) in state.log.entries.iter().enumerate() {
        let shape = RowShape {
            project: state.project.clone(),
            cell: GraphCell::at(&state.log.rows, index),
            lanes: lane_count,
            selected: selected == Some(index),
            columns: state.log.columns,
            can_fold: index == 0 && entry.parents.len() == 1,
            is_tip: index == 0,
            on_branch: matches!(
                state.status.as_ref().map(|status| &status.head),
                Some(transom_core::HeadState::OnBranch(_))
            ),
            current_branch: cur_branch.clone(),
        };
        rows.push(render_commit_row(tab_id, index, entry, &shape, &app, p));
    }
    if state.log.has_more && !state.log.loading {
        let more_app = app.clone();
        rows.push(
            h_flex()
                .justify_center()
                .py_2()
                .child(
                    Button::new("git-log-more")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("git.log.load_more").to_string())
                        .on_click(move |_, _, cx| {
                            more_app.update(cx, |this, cx| {
                                this.git_load_more_log(tab_id, cx);
                            });
                        }),
                )
                .into_any_element(),
        );
    }
    let context_columns = state.log.columns;
    let context_app = app.clone();
    v_flex()
        .w_full()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scrollbar()
        .children(rows)
        .context_menu(move |mut menu, _window, _cx| {
            let menu_app = context_app.clone();
            for column in [LogColumn::Author, LogColumn::Time] {
                let item_app = menu_app.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!(column.i18n_key()).to_string())
                        .checked(column.is_shown(&context_columns))
                        .on_click(move |_, _, cx| {
                            item_app.update(cx, |this, cx| {
                                this.git_toggle_log_column(tab_id, column, cx);
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

/// 一行的图形态：泳道数据 + 本页列数 + 是否选中 + 列开关 + 头指针态。
#[derive(Clone)]
struct RowShape {
    project: GitProject,
    cell: GraphCell,
    lanes: usize,
    selected: bool,
    columns: LogColumns,
    /// 头指针是否挂在分支上：游离头时「推送此前所有提交」没有目标分支。
    on_branch: bool,
    /// 这一行是否可被折叠：折叠只作用于分支头提交压进其父提交。
    can_fold: bool,
    /// 这一行是否就是分支头提交：它之上没有可重排的提交，不能当变基基线。
    is_tip: bool,
    /// 当前活跃分支名。
    current_branch: Option<String>,
}

/// 一行提交。
fn render_commit_row(
    tab_id: TabId,
    index: usize,
    entry: &transom_core::CommitEntry,
    shape: &RowShape,
    app: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let (cell, lane_count, selected, columns) =
        (&shape.cell, shape.lanes, shape.selected, shape.columns);
    // 菜单闭包要活过本次渲染，需要的字段一律先取成局部值。
    let project = shape.project.clone();
    let can_fold = shape.can_fold;
    let select_app = app.clone();
    let menu_app = app.clone();
    let oid = entry.oid.clone();
    let parents = entry.parents.clone();
    // 菜单闭包要活过本次渲染，因此把需要的字段先取成局部值，不留 `shape` 的借用。
    let on_branch = shape.on_branch;
    let is_tip = shape.is_tip;
    let current_branch = shape.current_branch.clone();
    h_flex()
        .id(ElementId::Name(format!("git-commit-{index}").into()))
        .h(px(ROW_HEIGHT))
        .items_center()
        .gap_2()
        .px_1()
        .cursor_pointer()
        .when(selected, |style| style.bg(p.list_active))
        .when(!selected, |style| style.hover(|el| el.bg(p.list_hover)))
        .child(render_graph_cells(cell, lane_count, p))
        // 概要列把「标题 + 分支标签」放进同一个弹性单元：标签若自成独立列，
        // 作者/时间就会与表头错位（表头只有 概要/作者/时间 三列）。
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .overflow_hidden()
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .truncate()
                        .text_sm()
                        .text_color(p.foreground)
                        .child(entry.subject.clone()),
                )
                .child(refs_line(&entry.refs, p)),
        )
        .when(columns.author, |row| {
            row.child(
                div()
                    .w(px(AUTHOR_WIDTH))
                    .overflow_hidden()
                    .truncate()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(entry.author_name.clone()),
            )
        })
        .when(columns.time, |row| {
            row.child(
                div()
                    .w(px(TIME_WIDTH))
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(relative_time(entry.author_time)),
            )
        })
        .on_click(move |_, _, cx| {
            select_app.update(cx, |this, cx| {
                this.git_select_commit_by_index(tab_id, index, cx);
            });
        })
        .context_menu(move |menu, _window, _cx| {
            render_commit_menu(
                menu,
                menu_app.clone(),
                CommitMenuTarget {
                    project: project.clone(),
                    tab_id,
                    oid: oid.clone(),
                    parents: parents.clone(),
                    on_branch,
                    can_fold,
                    is_tip,
                    current_branch: current_branch.clone(),
                },
            )
        })
        .into_any_element()
}

/// 引用装饰行（分支名 / 标签名徽标）。
fn refs_line(refs: &[transom_core::RefDecor], p: &Palette) -> AnyElement {
    if refs.is_empty() {
        return div().into_any_element();
    }
    h_flex()
        .gap_1()
        .flex_none()
        .children(
            refs.iter()
                .map(|decor| ref_chip(decor.kind, &decor.name, p)),
        )
        .into_any_element()
}

/// 泳道区：交给自绘层画节点与连线。
fn render_graph_cells(cell: &GraphCell, lanes: usize, p: &Palette) -> AnyElement {
    graph_paint::paint(cell.clone(), lanes, p.primary, p.primary.opacity(0.55))
}
