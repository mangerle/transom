//! 离线搁置项（Shelf）面板：IDE 独立离线补丁集管理。
//!
//! 设计原理（Why）：
//! - 形态对齐 IDEA 的「搁置（Shelf）」：与 Git Stash 严格区分，搁置是保存在本地磁盘
//!   的离线补丁集，不依赖 Git 仓库内部的 stash 栈；
//! - 支持一键将勾选变更生成补丁并从工作区抽离，也可随时将补丁应用（Apply）、
//!   弹回（Pop，即应用后删除）或彻底删除（Drop）；
//! - 即使切换了分支或在不同工作树中，离线补丁永不丢失。

use super::commit_modal::split_path;
use crate::app::AppShell;
use crate::git::types::GitDiffRequest;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId, GitSide, HeadState, ShelfEntry, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 搁置内联面板：提交页左栏的「搁置」页签内容。
pub(crate) fn render_git_shelf_panel(
    shell: &AppShell,
    tab_id: TabId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return div().into_any_element();
    };
        let (project, entries, expanded) = {
            let state = &workspace.read(cx).state;
            (
                state.project.clone(),
                state.shelves.clone(),
                state.expanded_shelf.clone(),
            )
        };

        let refresh_project = project.clone();
        let toolbar = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(
                Button::new("git-shelf-refresh")
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                    .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                    .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        crate::git::actions::git_refresh(this, &refresh_project, cx);
                        actions::git_reload_shelves(this, tab_id, cx);
                    })),
            )
            .child(
                Button::new("git-shelf-new")
                    .primary()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::PATCH)).small())
                    .label(rust_i18n::t!("git.shelf.create").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        actions::git_shelve_checked(this, tab_id, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!(
                        "{} ({})",
                        rust_i18n::t!("git.shelf.title"),
                        entries.len()
                    )),
            );

        let mut list = v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_1()
            .overflow_y_scrollbar();
        if entries.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("git.shelf.empty").to_string()),
            );
        }
        for entry in &entries {
            let is_expanded = expanded.as_deref() == Some(entry.id.as_str());
            list = list.child(render_shelf_entry_row(shell, tab_id, entry, is_expanded, p, cx));
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(list)
            .into_any_element()
}

/// 单个搁置项：标题行与展开的文件列表。
fn render_shelf_entry_row(
    shell: &AppShell,
    tab_id: TabId,
    entry: &ShelfEntry,
    expanded: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
        let entry_id = entry.id.clone();
        let toggle_id = entry.id.clone();
        let apply_id = entry.id.clone();
        let pop_id = entry.id.clone();
        let menu_id = entry.id.clone();

        let toggle_area = h_flex()
            .id(ElementId::Name(format!("git-shelf-head-{entry_id}").into()))
            .flex_1()
            .min_w_0()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .child(
                AppIconElement::new(if expanded {
                    AppIcon::Ui(UiIcon::CHEVRON_DOWN)
                } else {
                    AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
                })
                .xsmall()
                .text_color(p.muted_foreground),
            )
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::PATCH))
                    .xsmall()
                    .text_color(p.primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(p.foreground)
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("{} · {}", entry.branch, entry.created_at)),
                    ),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                actions::git_toggle_shelf_expanded(this, tab_id, &toggle_id, cx);
            }));

        let title = h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .child(toggle_area)
            .child(
                Button::new(ElementId::Name(
                    format!("git-shelf-apply-{entry_id}").into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                .accessibility_label(rust_i18n::t!("git.shelf.apply").to_string())
                .tooltip(rust_i18n::t!("git.shelf.apply").to_string())
                .on_click(cx.listener(move |this, _, _window, cx| {
                    actions::git_apply_shelf(this, tab_id, &apply_id, false, cx);
                })),
            )
            .child(
                Button::new(ElementId::Name(format!("git-shelf-pop-{entry_id}").into()))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                    .accessibility_label(rust_i18n::t!("git.shelf.pop").to_string())
                    .tooltip(rust_i18n::t!("git.shelf.pop").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        actions::git_apply_shelf(this, tab_id, &pop_id, true, cx);
                    })),
            )
            .context_menu({
                let app: Entity<AppShell> = cx.entity().clone();
                move |menu, _window, _cx| {
                    render_shelf_popup_menu(menu, app.clone(), tab_id, menu_id.clone())
                }
            });

        let mut rows = v_flex()
            .gap_0()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(title);
        if !expanded {
            return rows.into_any_element();
        }
        if entry.files.is_empty() {
            return rows
                .child(
                    div()
                        .pl_6()
                        .py_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("git.shelf.no_files").to_string()),
                )
                .into_any_element();
        }
        for file in &entry.files {
            rows = rows.child(render_shelf_file_row(shell, tab_id, entry, file, p, cx));
        }
        rows.into_any_element()
}

/// 搁置清单内单行文件项。
fn render_shelf_file_row(
    shell: &AppShell,
    tab_id: TabId,
    entry: &ShelfEntry,
    file_path: &str,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (name, dir_prefix) = split_path(file_path);
    let path_str = file_path.to_string();
    let selected = shell.state.git_workspace(tab_id).is_some_and(|workspace| {
            workspace
                .read(cx)
                .state
                .diff
                .as_ref()
                .is_some_and(|diff| diff.path == path_str)
        });
        let tooltip_path = path_str.clone();
        let file_for_diff = path_str.clone();
        let entry_for_diff = entry.clone();
        h_flex()
            .id(ElementId::Name(
                format!("git-shelf-file-{}-{path_str}", entry.id).into(),
            ))
            .h(px(22.0))
            .pl_6()
            .pr_2()
            .gap_2()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|style| style.bg(p.list_hover))
            .when(selected, |el| el.bg(p.list_active))
            .tooltip(move |window, cx| Tooltip::new(tooltip_path.clone()).build(window, cx))
            .child(
                AppIconElement::new(AppIcon::for_file(&path_str))
                    .with_rems(rems(crate::theme::tokens::control::ICON)),
            )
            .child(
                div()
                    .text_sm()
                    .flex_none()
                    .max_w(px(180.0))
                    .overflow_hidden()
                    .truncate()
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
                    .child(dir_prefix),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                actions::git_open_shelf_file_diff(this, tab_id, &entry_for_diff, &file_for_diff, cx);
            }))
            .into_any_element()
}

pub(crate) mod actions;
use actions::render_shelf_popup_menu;
