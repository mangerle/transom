//! 离线搁置项（Shelf）面板：IDE 独立离线补丁集管理。
//!
//! 设计原理（Why）：
//! - 形态对齐 IDEA 的「搁置（Shelf）」：与 Git Stash 严格区分，搁置是保存在本地磁盘
//!   的离线补丁集，不依赖 Git 仓库内部的 stash 栈；
//! - 支持一键将勾选变更生成补丁并从工作区抽离，也可随时将补丁应用（Apply）、
//!   弹回（Pop，即应用后删除）或彻底删除（Drop）；
//! - 即使切换了分支或在不同工作树中，离线补丁永不丢失。

use super::commit_modal::split_path;
use crate::git::GitWorkspace;
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
use transom_core::ShelfEntry;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 搁置内联面板：提交页左栏的「搁置」页签内容。
pub(crate) fn render_git_shelf_panel(
    workspace: Entity<GitWorkspace>,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let (entries, expanded) = {
        let state = &workspace.read(cx).state;
        (state.shelves.clone(), state.expanded_shelf.clone())
    };

    let refresh_ws = workspace.clone();
    let new_ws = workspace.clone();
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
                .on_click(move |_, _window, cx| {
                    refresh_ws.update(cx, |ws, cx| {
                        ws.refresh();
                        ws.reload_shelves(cx);
                    });
                }),
        )
        .child(
            Button::new("git-shelf-new")
                .primary()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::PATCH)).small())
                .label(rust_i18n::t!("git.shelf.create").to_string())
                .on_click(move |_, _window, cx| {
                    new_ws.update(cx, |ws, cx| ws.shelve_checked(cx));
                }),
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
        list = list.child(render_shelf_entry_row(
            &workspace,
            entry,
            is_expanded,
            p,
            cx,
        ));
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
    workspace: &Entity<GitWorkspace>,
    entry: &ShelfEntry,
    expanded: bool,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let entry_id = entry.id.clone();
    let toggle_ws = workspace.clone();
    let toggle_id = entry.id.clone();
    let apply_ws = workspace.clone();
    let apply_id = entry.id.clone();
    let pop_ws = workspace.clone();
    let pop_id = entry.id.clone();
    let menu_ws = workspace.clone();
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
        .on_click(move |_, _window, cx| {
            toggle_ws.update(cx, |ws, cx| ws.toggle_shelf_expanded(&toggle_id, cx));
        });

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
            .on_click(move |_, _window, cx| {
                apply_ws.update(cx, |ws, cx| ws.apply_shelf(&apply_id, false, cx));
            }),
        )
        .child(
            Button::new(ElementId::Name(format!("git-shelf-pop-{entry_id}").into()))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                .accessibility_label(rust_i18n::t!("git.shelf.pop").to_string())
                .tooltip(rust_i18n::t!("git.shelf.pop").to_string())
                .on_click(move |_, _window, cx| {
                    pop_ws.update(cx, |ws, cx| ws.apply_shelf(&pop_id, true, cx));
                }),
        )
        .context_menu(move |menu, _window, _cx| {
            render_shelf_popup_menu(menu, menu_ws.clone(), menu_id.clone())
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
        rows = rows.child(render_shelf_file_row(workspace, entry, file, p, cx));
    }
    rows.into_any_element()
}

/// 搁置清单内单行文件项。
fn render_shelf_file_row(
    workspace: &Entity<GitWorkspace>,
    entry: &ShelfEntry,
    file_path: &str,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let (name, dir_prefix) = split_path(file_path);
    let path_str = file_path.to_string();
    let selected = workspace
        .read(cx)
        .state
        .diff
        .as_ref()
        .is_some_and(|diff| diff.path == path_str);
    let tooltip_path = path_str.clone();
    let file_for_diff = path_str.clone();
    let entry_for_diff = entry.clone();
    let diff_ws = workspace.clone();
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
        .on_click(move |_, _window, cx| {
            diff_ws.update(cx, |ws, cx| {
                ws.open_shelf_file_diff(&entry_for_diff, &file_for_diff, cx)
            });
        })
        .into_any_element()
}

/// 搁置项右键菜单。
fn render_shelf_popup_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    entry_id: String,
) -> PopupMenu {
    let ws_apply = workspace.clone();
    let apply_id = entry_id.clone();
    let ws_pop = workspace.clone();
    let pop_id = entry_id.clone();
    let drop_id = entry_id;
    menu.item(menu_item(
        rust_i18n::t!("git.shelf.apply").to_string(),
        AppIcon::Ui(UiIcon::RUN),
        move |_, _, cx| {
            let id = apply_id.clone();
            ws_apply.update(cx, |ws, cx| ws.apply_shelf(&id, false, cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.shelf.pop").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let id = pop_id.clone();
            ws_pop.update(cx, |ws, cx| ws.apply_shelf(&id, true, cx));
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.shelf.drop").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            let id = drop_id.clone();
            workspace.update(cx, |ws, cx| ws.drop_shelf(&id, cx));
        },
    ))
}
