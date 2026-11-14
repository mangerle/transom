//! 公共 Diff 视图控件组：统一的主体渲染与工具条选项菜单。
//!
//! 设计原理（Why）：
//! - 将渲染细节（`Diff` 组件配置）与工具条按钮/下拉菜单（上/下一处差异、
//!   展开/收起上下文、并排/统一、忽略空白、高亮粒度、外观设置）抽离成纯UI控件；
//! - 宿主（Git 差异面板、SFTP 远程编辑器预览）只需提供 `Option<Entity<DiffState>>`、
//!   当前 `DiffViewOptions` 以及一个 `on_update` 回调，即可得到完全一致的交互与视觉体验。

use std::rc::Rc;

use gpui_kit::base::{Disableable, h_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::{
    Diff, DiffChangeIndicator, DiffHoverHighlight, DiffHunkSeparator, DiffState,
};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::{AppIconElement, UiIcon};

use super::types::{DiffHighlight, DiffUiAction, DiffViewOptions, IgnoreWhitespace};
use crate::widgets::menu_item;

/// 选项更新回调类型：修改 `DiffViewOptions` 的闭包会被派发到具体宿主的状态更新中。
pub(crate) type OptionsUpdater =
    Rc<dyn Fn(Box<dyn FnOnce(&mut DiffViewOptions) + 'static>, &mut Window, &mut App) + 'static>;

/// 渲染公共 `Diff` 主体控件，并统一注入配置选项。
pub(crate) fn render_diff_body(state: &Entity<DiffState>, options: DiffViewOptions) -> Diff {
    let show_colors = options.highlight != DiffHighlight::None;
    Diff::new(state)
        .header_visible(false)
        .line_number(options.show_line_numbers)
        .soft_wrap(options.wrap_lines)
        .syntax_highlight(true)
        .change_background(show_colors)
        .change_indicator(if show_colors {
            DiffChangeIndicator::Bars
        } else {
            DiffChangeIndicator::None
        })
        .hover_highlight(DiffHoverHighlight::Both)
        .hunk_separator(DiffHunkSeparator::LineInfo)
        .border_0()
}

/// 选择型菜单项：选中项使用勾选图标。
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

/// 渲染「上一处差异 / 下一处差异」导航按钮，直接驱动 `DiffState::previous_change` / `next_change`。
pub(crate) fn render_change_nav_buttons(
    id_prefix: &'static str,
    diff_state: Option<Entity<DiffState>>,
    has_changes: bool,
) -> AnyElement {
    let disabled = diff_state.is_none() || !has_changes;
    let prev_state = diff_state.clone();
    let next_state = diff_state;
    render_change_nav_with_handlers(
        id_prefix,
        disabled,
        Rc::new(move |_window, cx| {
            if let Some(state) = prev_state.as_ref() {
                state.update(cx, |s, cx| s.previous_change(cx));
            }
        }),
        Rc::new(move |_window, cx| {
            if let Some(state) = next_state.as_ref() {
                state.update(cx, |s, cx| s.next_change(cx));
            }
        }),
    )
}

/// 可定制点击回调的「上一处/下一处差异」导航按钮（用于跨文件差异推进）。
pub(crate) fn render_change_nav_with_handlers(
    id_prefix: &'static str,
    disabled: bool,
    on_prev: DiffUiAction,
    on_next: DiffUiAction,
) -> AnyElement {
    h_flex()
        .gap_0()
        .child(
            Button::new(format!("{id_prefix}-prev-diff"))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::CHEVRON_UP))
                .disabled(disabled)
                .accessibility_label(rust_i18n::t!("git.diff.prev_difference").to_string())
                .tooltip(format!(
                    "{} (Shift+F7)",
                    rust_i18n::t!("git.diff.prev_difference")
                ))
                .on_click(move |_, window, cx| on_prev(window, cx)),
        )
        .child(
            Button::new(format!("{id_prefix}-next-diff"))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::CHEVRON_DOWN))
                .disabled(disabled)
                .accessibility_label(rust_i18n::t!("git.diff.next_difference").to_string())
                .tooltip(format!(
                    "{} (F7)",
                    rust_i18n::t!("git.diff.next_difference")
                ))
                .on_click(move |_, window, cx| on_next(window, cx)),
        )
        .into_any_element()
}

/// 渲染「并排显示 / 统一视图」切换按钮组。
pub(crate) fn render_mode_toggle(
    id_prefix: &'static str,
    side_by_side: bool,
    on_update: OptionsUpdater,
) -> AnyElement {
    let split_cb = on_update.clone();
    let unified_cb = on_update;
    h_flex()
        .id(format!("{id_prefix}-mode"))
        .gap_0()
        .child(
            Button::new(format!("{id_prefix}-mode-split"))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::EDITOR_PREVIEW))
                .tooltip(rust_i18n::t!("git.diff.side_by_side").to_string())
                .accessibility_label(rust_i18n::t!("git.diff.side_by_side").to_string())
                .selected(side_by_side)
                .on_click(move |_, window, cx| {
                    split_cb(Box::new(|options| options.side_by_side = true), window, cx);
                }),
        )
        .child(
            Button::new(format!("{id_prefix}-mode-unified"))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::EDITOR_ONLY))
                .tooltip(rust_i18n::t!("git.diff.unified").to_string())
                .accessibility_label(rust_i18n::t!("git.diff.unified").to_string())
                .selected(!side_by_side)
                .on_click(move |_, window, cx| {
                    unified_cb(Box::new(|options| options.side_by_side = false), window, cx);
                }),
        )
        .into_any_element()
}

fn ignore_level_key(level: IgnoreWhitespace) -> &'static str {
    match level {
        IgnoreWhitespace::None => "git.diff.ignore_none",
        IgnoreWhitespace::Trim => "git.diff.ignore_trim",
        IgnoreWhitespace::All => "git.diff.ignore_all",
        IgnoreWhitespace::BlankLines => "git.diff.ignore_blank",
    }
}

fn highlight_level_key(level: DiffHighlight) -> &'static str {
    match level {
        DiffHighlight::Line => "git.diff.highlight_line",
        DiffHighlight::Word => "git.diff.highlight_word",
        DiffHighlight::None => "git.diff.highlight_none",
    }
}

fn appearance_tooltip(options: DiffViewOptions) -> String {
    let mut active = Vec::with_capacity(3);
    if options.show_line_numbers {
        active.push(rust_i18n::t!("git.diff.show_line_numbers").to_string());
    }
    if options.wrap_lines {
        active.push(rust_i18n::t!("git.diff.wrap_lines").to_string());
    }
    if options.collapse_unchanged {
        active.push(rust_i18n::t!("git.diff.collapse_unchanged").to_string());
    }
    if active.is_empty() {
        rust_i18n::t!("git.diff.settings").to_string()
    } else {
        rust_i18n::t!(
            "git.diff.control_tooltip_with_value",
            title = rust_i18n::t!("git.diff.settings").to_string(),
            value = active.join(", ")
        )
        .to_string()
    }
}

/// 渲染「忽略空白差异」4 档菜单。
pub(crate) fn render_ignore_menu(
    id_prefix: &'static str,
    current: IgnoreWhitespace,
    on_update: OptionsUpdater,
) -> AnyElement {
    let is_active = current != IgnoreWhitespace::None;
    let tooltip = rust_i18n::t!(
        "git.diff.control_tooltip_with_value",
        title = rust_i18n::t!("git.diff.ignore_title").to_string(),
        value = rust_i18n::t!(ignore_level_key(current)).to_string()
    )
    .to_string();
    Button::new(format!("{id_prefix}-ignore"))
        .ghost()
        .xsmall()
        .child(AppIconElement::new(UiIcon::TOGGLE_VISIBILITY))
        .when(is_active, |btn| btn.secondary())
        .accessibility_label(tooltip.clone())
        .tooltip(tooltip)
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for (key, level) in [
                ("git.diff.ignore_none", IgnoreWhitespace::None),
                ("git.diff.ignore_trim", IgnoreWhitespace::Trim),
                ("git.diff.ignore_all", IgnoreWhitespace::All),
                ("git.diff.ignore_blank", IgnoreWhitespace::BlankLines),
            ] {
                let cb = on_update.clone();
                let checked = current == level;
                menu = menu.item(choice_item(
                    rust_i18n::t!(key).to_string(),
                    checked,
                    move |_, window, cx| {
                        cb(
                            Box::new(move |options| {
                                options.ignore_whitespace = level;
                            }),
                            window,
                            cx,
                        );
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 渲染「高亮显示差异」3 档菜单（整行 / 单词 / 无）。
pub(crate) fn render_highlight_menu(
    id_prefix: &'static str,
    current: DiffHighlight,
    on_update: OptionsUpdater,
) -> AnyElement {
    let is_active = current != DiffHighlight::default();
    let tooltip = rust_i18n::t!(
        "git.diff.control_tooltip_with_value",
        title = rust_i18n::t!("git.diff.highlight_title").to_string(),
        value = rust_i18n::t!(highlight_level_key(current)).to_string()
    )
    .to_string();
    Button::new(format!("{id_prefix}-highlight"))
        .ghost()
        .xsmall()
        .child(AppIconElement::new(UiIcon::HIGHLIGHT))
        .when(is_active, |btn| btn.secondary())
        .accessibility_label(tooltip.clone())
        .tooltip(tooltip)
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for (key, level) in [
                ("git.diff.highlight_line", DiffHighlight::Line),
                ("git.diff.highlight_word", DiffHighlight::Word),
                ("git.diff.highlight_none", DiffHighlight::None),
            ] {
                let cb = on_update.clone();
                let checked = current == level;
                menu = menu.item(choice_item(
                    rust_i18n::t!(key).to_string(),
                    checked,
                    move |_, window, cx| {
                        cb(
                            Box::new(move |options| {
                                options.highlight = level;
                            }),
                            window,
                            cx,
                        );
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 渲染「外观」子菜单：显示行号 / 自动换行 / 折叠未变更上下文。
pub(crate) fn render_appearance_menu(
    id_prefix: &'static str,
    options: DiffViewOptions,
    on_update: OptionsUpdater,
) -> AnyElement {
    let defaults = DiffViewOptions::default();
    let is_custom = options.show_line_numbers != defaults.show_line_numbers
        || options.wrap_lines != defaults.wrap_lines
        || options.collapse_unchanged != defaults.collapse_unchanged;
    let tooltip = appearance_tooltip(options);
    Button::new(format!("{id_prefix}-appearance"))
        .ghost()
        .xsmall()
        .child(AppIconElement::new(UiIcon::LAYOUT))
        .when(is_custom, |btn| btn.secondary())
        .accessibility_label(tooltip.clone())
        .tooltip(tooltip)
        .dropdown_menu(move |menu, _window, _cx| {
            let numbers_cb = on_update.clone();
            let wrap_cb = on_update.clone();
            let collapse_cb = on_update.clone();
            let numbers_on = options.show_line_numbers;
            let wrap_on = options.wrap_lines;
            let collapse_on = options.collapse_unchanged;
            menu.item(choice_item(
                rust_i18n::t!("git.diff.show_line_numbers").to_string(),
                numbers_on,
                move |_, window, cx| {
                    numbers_cb(
                        Box::new(move |opts| {
                            opts.show_line_numbers = !numbers_on;
                        }),
                        window,
                        cx,
                    );
                },
            ))
            .item(choice_item(
                rust_i18n::t!("git.diff.wrap_lines").to_string(),
                wrap_on,
                move |_, window, cx| {
                    wrap_cb(
                        Box::new(move |opts| {
                            opts.wrap_lines = !wrap_on;
                        }),
                        window,
                        cx,
                    );
                },
            ))
            .item(choice_item(
                rust_i18n::t!("git.diff.collapse_unchanged").to_string(),
                collapse_on,
                move |_, window, cx| {
                    collapse_cb(
                        Box::new(move |opts| {
                            opts.collapse_unchanged = !collapse_on;
                        }),
                        window,
                        cx,
                    );
                },
            ))
        })
        .into_any_element()
}

/// 渲染标准显示控制菜单组（并排/统一 + 忽略空白 + 高亮粒度 + 外观）。
pub(crate) fn render_option_controls(
    id_prefix: &'static str,
    options: DiffViewOptions,
    on_update: OptionsUpdater,
) -> AnyElement {
    h_flex()
        .gap_0p5()
        .items_center()
        .child(render_mode_toggle(
            id_prefix,
            options.side_by_side,
            on_update.clone(),
        ))
        .child(render_ignore_menu(
            id_prefix,
            options.ignore_whitespace,
            on_update.clone(),
        ))
        .child(render_highlight_menu(
            id_prefix,
            options.highlight,
            on_update.clone(),
        ))
        .child(render_appearance_menu(id_prefix, options, on_update))
        .into_any_element()
}
