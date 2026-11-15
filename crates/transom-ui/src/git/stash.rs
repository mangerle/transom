//! 贮藏（Stash）面板：原生 Git Stash 栈管理。
//!
//! 设计原理（Why）：
//! - 形态对齐 IDEA 的 Git Stash：一列贮藏栈项，每项可应用 / 弹出 / 丢弃，
//!   顶部一个「新建贮藏」入口；
//! - 「应用」与「弹出」分列两个按钮：前者保留栈项、可反复应用到多个分支，
//!   后者会把栈项从 stash 栈移除——界面必须区分明确；
//! - 删除走独立确认，不与列表点击混在一起。

use super::commit_modal::split_path;
use crate::git::{GitWorkspace, change_accent_color};
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
use transom_core::GitAction;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 贮藏内联面板：提交页左栏的「贮藏」页签内容。
pub(crate) fn render_git_stash_panel(
    workspace: Entity<GitWorkspace>,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let (entries, expanded, files, loading) = {
        let state = &workspace.read(cx).state;
        (
            state.stashes.clone(),
            state.expanded_stash.clone(),
            state.stash_files.clone(),
            state.stash_files_loading,
        )
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
            Button::new("git-stash-refresh")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                .on_click(move |_, _window, cx| {
                    refresh_ws.update(cx, |ws, cx| {
                        ws.refresh();
                        ws.reload_stashes(cx);
                    });
                }),
        )
        .child(
            Button::new("git-stash-new")
                .primary()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::PATCH)).small())
                .label(rust_i18n::t!("git.stash.create").to_string())
                .on_click(move |_, _window, cx| {
                    new_ws.update(cx, |ws, cx| ws.stash_checked(cx));
                }),
        )
        .child(div().flex_1())
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(format!(
                    "{} ({})",
                    rust_i18n::t!("git.stash.title"),
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
                .child(rust_i18n::t!("git.stash.empty").to_string()),
        );
    }
    for entry in entries {
        let shown = if expanded.as_deref() == Some(entry.selector.as_str()) {
            files.as_slice()
        } else {
            &[]
        };
        let view = StashRowView {
            workspace: &workspace,
            selector: &entry.selector,
            display: entry.display(),
            expanded: expanded.as_deref() == Some(entry.selector.as_str()),
            loading,
            files: shown,
        };
        list = list.child(stash_row(&view, p, cx));
    }

    v_flex()
        .w_full()
        .flex_1()
        .min_h_0()
        .child(toolbar)
        .child(list)
        .into_any_element()
}

/// 一条贮藏项：标题行 + 展开后的文件清单。
fn stash_row(view: &StashRowView<'_>, p: &Palette, cx: &App) -> AnyElement {
    let selector = view.selector.to_string();
    let toggle_ws = view.workspace.clone();
    let toggle_target = selector.clone();
    let toggle_area = h_flex()
        .id(ElementId::Name(format!("git-stash-head-{selector}").into()))
        .flex_1()
        .min_w_0()
        .gap_2()
        .items_center()
        .cursor_pointer()
        .child(
            AppIconElement::new(if view.expanded {
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
                        .child(view.display.to_string()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(selector.clone()),
                ),
        )
        .on_click(move |_, _window, cx| {
            toggle_ws.update(cx, |ws, cx| ws.toggle_stash_files(&toggle_target, cx));
        });
    let apply_ws = view.workspace.clone();
    let apply_target = selector.clone();
    let pop_ws = view.workspace.clone();
    let pop_target = selector.clone();
    let menu_ws = view.workspace.clone();
    let menu_target = selector;
    let title = h_flex()
        .px_2()
        .py_1p5()
        .gap_2()
        .items_center()
        .child(toggle_area)
        .child(
            Button::new(ElementId::Name(
                format!("git-stash-apply-{apply_target}").into(),
            ))
            .ghost()
            .xsmall()
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
            .accessibility_label(rust_i18n::t!("git.stash.apply").to_string())
            .tooltip(rust_i18n::t!("git.stash.apply").to_string())
            .on_click(move |_, _window, cx| {
                let target = apply_target.clone();
                apply_ws.update(cx, |ws, cx| {
                    ws.run_stash_action(target, GitAction::StashApply, cx);
                });
            }),
        )
        .child(
            Button::new(ElementId::Name(
                format!("git-stash-pop-{pop_target}").into(),
            ))
            .ghost()
            .xsmall()
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
            .accessibility_label(rust_i18n::t!("git.stash.pop").to_string())
            .tooltip(rust_i18n::t!("git.stash.pop").to_string())
            .on_click(move |_, _window, cx| {
                let target = pop_target.clone();
                pop_ws.update(cx, |ws, cx| {
                    ws.run_stash_action(target, GitAction::StashPop, cx);
                });
            }),
        )
        .context_menu(move |menu, _window, _cx| {
            render_stash_menu(menu, menu_ws.clone(), menu_target.clone())
        });

    let mut rows = v_flex()
        .gap_0()
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(title);
    if !view.expanded {
        return rows.into_any_element();
    }
    if view.files.is_empty() {
        let text = if view.loading {
            rust_i18n::t!("git.stash.files_loading").to_string()
        } else {
            rust_i18n::t!("git.stash.no_files").to_string()
        };
        return rows
            .child(
                div()
                    .pl_6()
                    .py_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(text),
            )
            .into_any_element();
    }
    for file in view.files {
        rows = rows.child(stash_file_row(view, file, p, cx));
    }
    rows.into_any_element()
}

/// 贮藏清单里的一行文件：点击即看「贮藏前 vs 贮藏后」的差异。
fn stash_file_row(
    view: &StashRowView<'_>,
    file: &transom_core::StashFile,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let visual = crate::git::change_visual(file.change);
    let (name, dir_prefix) = split_path(&file.path);
    let selector = view.selector.to_string();
    let open_file = file.clone();
    let diff_ws = view.workspace.clone();
    let selected = view
        .workspace
        .read(cx)
        .state
        .diff
        .as_ref()
        .is_some_and(|diff| diff.path == file.path);
    let tooltip_path = file.path.clone();
    h_flex()
        .id(ElementId::Name(
            format!("git-stash-file-{selector}-{}", file.path).into(),
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
            AppIconElement::new(visual.icon)
                .xsmall()
                .text_color(change_accent_color(visual.accent, p)),
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
        .child(
            file.old_path
                .as_ref()
                .map(|old| {
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(old.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
        )
        .on_click(move |_, _window, cx| {
            diff_ws.update(cx, |ws, cx| ws.open_stash_diff(&selector, &open_file, cx));
        })
        .into_any_element()
}

/// 贮藏行的展示参数包。
struct StashRowView<'a> {
    workspace: &'a Entity<GitWorkspace>,
    selector: &'a str,
    display: &'a str,
    expanded: bool,
    loading: bool,
    files: &'a [transom_core::StashFile],
}

/// 贮藏项右键菜单。
fn render_stash_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    selector: String,
) -> PopupMenu {
    let drop_ws = workspace.clone();
    let drop_selector = selector.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.stash.apply").to_string(),
        AppIcon::Ui(UiIcon::RUN),
        {
            let ws_apply = workspace.clone();
            let item_selector = selector.clone();
            move |_, _, cx| {
                let selector = item_selector.clone();
                ws_apply.update(cx, |ws, cx| {
                    ws.run_stash_action(selector, GitAction::StashApply, cx);
                });
            }
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.stash.pop").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let selector = selector.clone();
            workspace.update(cx, |ws, cx| {
                ws.run_stash_action(selector, GitAction::StashPop, cx);
            });
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.stash.drop").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            let selector = drop_selector.clone();
            drop_ws.update(cx, |ws, cx| {
                ws.run_stash_action(selector, GitAction::StashDrop, cx);
            });
        },
    ))
}
