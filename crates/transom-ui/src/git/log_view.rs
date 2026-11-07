//! 日志工作区的三个面板：引用栏、提交表、详情与文件清单。
//!
//! 设计原理（Why）：
//! - 全部渲染函数都是「只读状态 + 实体句柄」的自由函数：标签页状态存在
//!   `workspace.tabs` 里，若渲染期间持有 `&mut GitWorkspaceState` 又去调用
//!   `AppShell` 的方法，借用检查器直接判死。句柄回调把变更推迟到点击时刻，
//!   既解开借用，也让每个面板可独立测试；
//! - 引用栏按 IDEA 分组（本地 / 远程分支 / 标签），默认只展开「本地」：分支多
//!   的仓库全展开会把可视区吃光；
//! - 提交图由 `graph_paint` 在 canvas 上自绘节点与贝塞尔连线：gpui 的 div 布局
//!   画不出跨行的分叉连线，用竖线加圆点近似会在合并提交上画错。
//! - 详情区点文件才发差异请求：一次合并提交可能上百个文件，预取全部差异会
//!   瞬间打满子进程队列；
//! - 本模块只留「过滤条 + 详情」与对外入口，提交表、引用栏、右键菜单各自成子模块
//!   （`log_table` / `log_refs` / `log_menus`）：单文件堆到千行以上后，对照截图逐项
//!   核查要在一屏里滚半个文件，拆开也让每一类的改动只碰一个文件。对外仍从这里
//!   再导出，调用方的引用路径不变。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

mod files_tree;
mod log_details;
mod log_refs;
mod log_table;
pub(crate) mod menus;
mod worktrees_group;

use menus::{render_date_menu, render_path_menu};

// 外部（工作区/其它面板）仍按 `crate::git::log_view::*` 引用日志面板的主入口，
// 拆模块只是把实现挪到同级的子模块里，不改变对外的形状。
pub(crate) use log_details::render_right_panel;
pub(crate) use log_refs::render_refs_panel;
pub(crate) use log_table::render_log_table;

type State = crate::git::GitWorkspaceState;

/// 日志过滤条：对应 IDEA 日志页顶部那一排（检索框、`.*`/`Cc` 开关、
/// 分支 / 用户 / 日期 / 路径下拉）。
pub(crate) fn render_filter_bar(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> AnyElement {
    let branch_names: Vec<String> = state
        .branches
        .iter()
        .filter(|branch| !branch.remote)
        .map(|branch| branch.name.clone())
        .collect();
    let scope_label = match &state.query.scope {
        transom_core::LogScope::CurrentBranch => rust_i18n::t!("git.scope.current").to_string(),
        transom_core::LogScope::AllRefs => rust_i18n::t!("git.scope.all").to_string(),
        transom_core::LogScope::Ref(name) => name.clone(),
    };
    let date_label = match state.query.since.as_deref() {
        None => rust_i18n::t!("git.filter.date_all").to_string(),
        Some(since) => since.to_string(),
    };
    let path_label = match state.query.paths.first() {
        None => rust_i18n::t!("git.filter.path_all").to_string(),
        Some(path) => path.clone(),
    };
    let search_state = state.search_input.clone();
    let author_state = state.author_input.clone();
    h_flex()
        .id("git-filter-bar")
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .flex_none()
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(
            Button::new("git-filter-scope")
                .ghost()
                .xsmall()
                .icon(IconName::GitBranch)
                .label(scope_label)
                .dropdown_menu({
                    let menu_app = app.clone();
                    let menu_branches = branch_names.clone();
                    move |menu, _window, _cx| {
                        let current_app = menu_app.clone();
                        let branch_names = menu_branches.clone();
                        let mut menu = menu
                            .item(menu_item(
                                rust_i18n::t!("git.scope.current").to_string(),
                                IconName::Circle,
                                move |_, _, cx| {
                                    current_app.update(cx, |this, cx| {
                                        this.git_set_log_scope(
                                            tab_id,
                                            transom_core::LogScope::CurrentBranch,
                                            cx,
                                        );
                                    });
                                },
                            ))
                            .item(menu_item(
                                rust_i18n::t!("git.scope.all").to_string(),
                                IconName::Circle,
                                {
                                    let all_app = menu_app.clone();
                                    move |_, _, cx| {
                                        all_app.update(cx, |this, cx| {
                                            this.git_set_log_scope(
                                                tab_id,
                                                transom_core::LogScope::AllRefs,
                                                cx,
                                            );
                                        });
                                    }
                                },
                            ))
                            .item(PopupMenuItem::separator());
                        for name in branch_names {
                            let item_app = menu_app.clone();
                            menu = menu.item(menu_item(
                                name.clone(),
                                IconName::GitBranch,
                                move |_, _, cx| {
                                    let name = name.clone();
                                    item_app.update(cx, |this, cx| {
                                        this.git_set_log_scope(
                                            tab_id,
                                            transom_core::LogScope::Ref(name),
                                            cx,
                                        );
                                    });
                                },
                            ));
                        }
                        menu
                    }
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&search_state).xsmall()),
        )
        .child(div().w(px(140.0)).child(Input::new(&author_state).xsmall()))
        .child(
            Button::new("git-filter-regex")
                .ghost()
                .xsmall()
                .label(".*")
                .when(state.query.regex, |btn| btn.primary())
                .on_click({
                    let click_app = app.clone();
                    move |_, _, cx| {
                        click_app.update(cx, |this, cx| {
                            this.git_toggle_filter_flag(tab_id, true, cx);
                        });
                    }
                }),
        )
        .child(
            Button::new("git-filter-case")
                .ghost()
                .xsmall()
                .label("Cc")
                .when(state.query.case_sensitive, |btn| btn.primary())
                .on_click({
                    let click_app = app.clone();
                    move |_, _, cx| {
                        click_app.update(cx, |this, cx| {
                            this.git_toggle_filter_flag(tab_id, false, cx);
                        });
                    }
                }),
        )
        .child(
            Button::new("git-filter-date")
                .ghost()
                .xsmall()
                .icon(IconName::Calendar)
                .label(date_label)
                .dropdown_menu({
                    let date_app = app.clone();
                    move |menu, _window, _cx| render_date_menu(menu, date_app.clone(), tab_id)
                }),
        )
        .child(
            Button::new("git-filter-path")
                .ghost()
                .xsmall()
                .icon(IconName::File)
                .label(path_label)
                .dropdown_menu({
                    let path_app = app.clone();
                    let paths = state.tracked_paths.clone();
                    move |menu, _window, _cx| {
                        render_path_menu(menu, path_app.clone(), tab_id, paths.clone())
                    }
                }),
        )
        .child(
            Button::new("git-filter-rebase-abort")
                .ghost()
                .xsmall()
                .icon(IconName::CircleSlash)
                .tooltip(rust_i18n::t!("git.menu.rebase_abort").to_string())
                .on_click({
                    let abort_app = app.clone();
                    move |_, _, cx| {
                        abort_app.update(cx, |this, cx| {
                            this.git_history_on_current(
                                crate::git::actions::GitHistoryAction::RebaseAbort,
                                cx,
                            );
                        });
                    }
                }),
        )
        .child(
            Button::new("git-filter-apply")
                .ghost()
                .xsmall()
                .icon(IconName::Search)
                .on_click({
                    let apply_app = app.clone();
                    move |_, _, cx| {
                        apply_app.update(cx, |this, cx| {
                            this.git_apply_filter_inputs(tab_id, cx);
                        });
                    }
                }),
        )
        .into_any_element()
}

fn placeholder(text: String, p: &Palette) -> AnyElement {
    placeholder_colored(text, p.muted_foreground)
}

fn placeholder_colored(text: String, color: Hsla) -> AnyElement {
    div()
        .p_3()
        .text_sm()
        .text_color(color)
        .child(text)
        .into_any_element()
}
