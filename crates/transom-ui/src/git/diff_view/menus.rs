//! 差异工具条的四个菜单：模式、忽略空白、高亮粒度、外观与齿轮项。
//!
//! 设计原理（Why）：这些菜单逐项对应 IDEA 差异窗口的工具条；集中一处便于对照
//! 截图核查「哪些开关存在、默认值是什么」，也让父模块只负责拼装。

use crate::app::AppShell;
use crate::git::types::{DiffHighlight, GitDiffOptions, IgnoreWhitespace};
use crate::widgets::menu_item;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;
use transom_ui_icons::{AppIconElement, UiIcon};

/// 选择型菜单项：选中项使用 New UI 勾选图标，未选中项不画伪单选圆点。
fn choice_item(
    label: String,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    if checked {
        menu_item(label, UiIcon::CHECK, on_click)
    } else {
        PopupMenuItem::new(label).on_click(on_click)
    }
}

/// 并排 / 统一切换。
pub(crate) fn render_mode_toggle(
    app: Entity<AppShell>,
    tab_id: TabId,
    side_by_side: bool,
) -> AnyElement {
    h_flex()
        .id("git-diff-mode")
        .gap_0()
        .child(
            Button::new("git-diff-mode-split")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::EDITOR_PREVIEW).small())
                .accessibility_label("Side-by-side diff")
                .when(side_by_side, |btn| btn.primary())
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        set_options(&app, tab_id, |options| options.side_by_side = true, cx);
                    }
                }),
        )
        .child(
            Button::new("git-diff-mode-unified")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::EDITOR_ONLY).small())
                .accessibility_label("Unified diff")
                .when(!side_by_side, |btn| btn.primary())
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        set_options(&app, tab_id, |options| options.side_by_side = false, cx);
                    }
                }),
        )
        .into_any_element()
}

pub(crate) fn set_options(
    app: &Entity<AppShell>,
    tab_id: TabId,
    update: impl FnOnce(&mut GitDiffOptions),
    cx: &mut App,
) {
    app.update(cx, |this, cx| {
        this.git_update_diff_options(tab_id, update, cx);
    });
}

/// 「忽略差异」档位菜单。
pub(crate) fn render_ignore_menu(
    app: Entity<AppShell>,
    tab_id: TabId,
    current: IgnoreWhitespace,
) -> AnyElement {
    Button::new("git-diff-ignore")
        .ghost()
        .xsmall()
        .icon(UiIcon::HIDE)
        .accessibility_label(rust_i18n::t!("git.diff.ignore_title").to_string())
        .tooltip(rust_i18n::t!("git.diff.ignore_title").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for (key, level) in [
                ("git.diff.ignore_none", IgnoreWhitespace::None),
                ("git.diff.ignore_trim", IgnoreWhitespace::Trim),
                ("git.diff.ignore_all", IgnoreWhitespace::All),
                ("git.diff.ignore_blank", IgnoreWhitespace::BlankLines),
            ] {
                let item_app = app.clone();
                let checked = current == level;
                menu = menu.item(choice_item(
                    rust_i18n::t!(key).to_string(),
                    checked,
                    move |_, _, cx| {
                        set_options(
                            &item_app,
                            tab_id,
                            |options| {
                                options.ignore_whitespace = level;
                            },
                            cx,
                        );
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 「高亮显示差异」档位菜单。
pub(crate) fn render_highlight_menu(
    app: Entity<AppShell>,
    tab_id: TabId,
    current: DiffHighlight,
) -> AnyElement {
    Button::new("git-diff-highlight")
        .ghost()
        .xsmall()
        .icon(UiIcon::HIGHLIGHT)
        .accessibility_label(rust_i18n::t!("git.diff.highlight_title").to_string())
        .tooltip(rust_i18n::t!("git.diff.highlight_title").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for (key, level) in [
                ("git.diff.highlight_line", DiffHighlight::Line),
                ("git.diff.highlight_word", DiffHighlight::Word),
                ("git.diff.highlight_none", DiffHighlight::None),
            ] {
                let item_app = app.clone();
                let checked = current == level;
                menu = menu.item(choice_item(
                    rust_i18n::t!(key).to_string(),
                    checked,
                    move |_, _, cx| {
                        set_options(
                            &item_app,
                            tab_id,
                            |options| {
                                options.highlight = level;
                            },
                            cx,
                        );
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 「外观」子菜单：显示行号 / 自动换行。
pub(crate) fn render_appearance_menu(
    app: Entity<AppShell>,
    tab_id: TabId,
    options: GitDiffOptions,
) -> AnyElement {
    Button::new("git-diff-appearance")
        .ghost()
        .xsmall()
        .icon(UiIcon::EDITOR_PREVIEW)
        .accessibility_label(rust_i18n::t!("git.diff.settings").to_string())
        .tooltip(rust_i18n::t!("git.diff.settings").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            let numbers_app = app.clone();
            let wrap_app = app.clone();
            let numbers_on = options.show_line_numbers;
            let wrap_on = options.wrap_lines;
            menu.item(choice_item(
                rust_i18n::t!("git.diff.show_line_numbers").to_string(),
                numbers_on,
                move |_, _, cx| {
                    set_options(
                        &numbers_app,
                        tab_id,
                        |options| {
                            options.show_line_numbers = !numbers_on;
                        },
                        cx,
                    );
                },
            ))
            .item(choice_item(
                rust_i18n::t!("git.diff.wrap_lines").to_string(),
                wrap_on,
                move |_, _, cx| {
                    set_options(
                        &wrap_app,
                        tab_id,
                        |options| {
                            options.wrap_lines = !wrap_on;
                        },
                        cx,
                    );
                },
            ))
        })
        .into_any_element()
}

/// 齿轮菜单：当前还做不到的能力按 IDEA 的原条目置灰呈现。
pub(crate) fn render_settings_menu() -> AnyElement {
    Button::new("git-diff-gear")
        .ghost()
        .xsmall()
        .icon(UiIcon::SETTINGS)
        .accessibility_label(rust_i18n::t!("common.more").to_string())
        .tooltip(rust_i18n::t!("common.more").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            menu.item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.align_changes").to_string())
                    .disabled(true),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.sync_scroll").to_string())
                    .disabled(true),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.open_in_window").to_string())
                    .disabled(true),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.all_files").to_string()).disabled(true),
            )
        })
        .into_any_element()
}
