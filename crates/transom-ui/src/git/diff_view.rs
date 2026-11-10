//! 差异视图：工具条 + 并排/统一两种行渲染。
//!
//! 设计原理（Why）：
//! - 两侧文本由 core 的 `DiffContent` 契约给出，本模块只做「按行配对 + 上色」，
//!   不碰 git；这样远程文件差异与本地 git 差异共用同一套渲染；
//! - 并排模式把连续的删除与插入按行配对，纯删除/纯插入补空白侧：与 IDEA 的
//!   对齐口径一致，逐行错位会让读者看不出改动范围；
//! - 词级高亮默认开启：整行染色的差异在大段重写里等于没信息，只有词级高亮
//!   能指出改在哪。

mod menus;
mod rows;
mod tabs;

use menus::{
    render_appearance_menu, render_highlight_menu, render_ignore_menu, render_mode_toggle,
    render_settings_menu,
};
use rows::{build_rows, render_row};

use crate::app::AppShell;
use crate::git::GitWorkspaceState as State;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::base::{ScrollBounce, h_flex, v_flex, v_virtual_list};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::Scrollbar;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::TabId;

/// 行高与行号列宽（px）。
pub(crate) const LINE_HEIGHT: f32 = 20.0;

pub(crate) const GUTTER_WIDTH: f32 = 46.0;

/// 一行的语义。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Added,
    Removed,
    Paired,
    Context,
}

/// 一行里的一段文本（词级高亮时会被切成多段）。
pub(crate) struct Segment {
    text: String,
    changed: bool,
}

/// 一侧的单元格。
#[derive(Clone)]
pub(crate) struct Cell {
    number: usize,
    text: String,
}

/// 一行差异（并排模式下左右各一格）。
#[derive(Clone)]
pub(crate) struct Row {
    left: Option<Cell>,
    right: Option<Cell>,
    tone: Tone,
}

/// 渲染差异面板。
pub(crate) fn render(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> AnyElement {
    let Some(diff) = state.active_diff() else {
        return placeholder(rust_i18n::t!("git.diff.none").to_string(), p);
    };
    let options = state.diff_options;
    let tab_bar = tabs::render_diff_tab_bar(state, app.clone(), tab_id, p);

    let total_tabs = state.diff_tabs.len();
    let current_tab_idx = state.active_diff_tab.unwrap_or(0);
    let prev_app = app.clone();
    let next_app = app.clone();

    let (rows, diff_summary) = match diff.content.as_ref() {
        Some(content) if !content.binary => {
            let r = build_rows(&content.old_text, &content.new_text, &options);
            let count = r.iter().filter(|row| row.tone != Tone::Context).count();
            let summary = rust_i18n::t!("git.diff.differences_count", count = count).to_string();
            (Some(r), Some(summary))
        }
        _ => (None, None),
    };

    let file_count_text = if total_tabs > 1 {
        rust_i18n::t!(
            "git.diff.files_indicator",
            current = current_tab_idx + 1,
            total = total_tabs
        )
        .to_string()
    } else {
        rust_i18n::t!("git.diff.single_file_indicator").to_string()
    };

    let header = h_flex()
        .id("git-diff-header")
        .w_full()
        .h(px(26.0))
        .flex_none()
        .px_2()
        .gap_1p5()
        .items_center()
        .text_xs()
        .bg(p.background)
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(
            Button::new("git-diff-prev-diff")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronUp)
                .accessibility_label(rust_i18n::t!("git.diff.prev_difference").to_string())
                .tooltip(rust_i18n::t!("git.diff.prev_difference").to_string()),
        )
        .child(
            Button::new("git-diff-next-diff")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronDown)
                .accessibility_label(rust_i18n::t!("git.diff.next_difference").to_string())
                .tooltip(rust_i18n::t!("git.diff.next_difference").to_string()),
        )
        .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
        .child(
            Button::new("git-diff-prev-file")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronLeft)
                .accessibility_label(rust_i18n::t!("git.diff.prev_file").to_string())
                .tooltip(rust_i18n::t!("git.diff.prev_file").to_string())
                .disabled(current_tab_idx == 0)
                .on_click(move |_, _, cx| {
                    if current_tab_idx > 0 {
                        prev_app.update(cx, |this, cx| {
                            this.git_select_diff_tab(tab_id, current_tab_idx - 1, cx);
                        });
                    }
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(file_count_text),
        )
        .child(
            Button::new("git-diff-next-file")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronRight)
                .accessibility_label(rust_i18n::t!("git.diff.next_file").to_string())
                .tooltip(rust_i18n::t!("git.diff.next_file").to_string())
                .disabled(total_tabs <= 1 || current_tab_idx + 1 >= total_tabs)
                .on_click(move |_, _, cx| {
                    if current_tab_idx + 1 < total_tabs {
                        next_app.update(cx, |this, cx| {
                            this.git_select_diff_tab(tab_id, current_tab_idx + 1, cx);
                        });
                    }
                }),
        )
        .child(div().flex_1())
        .children(diff_summary.map(|s| {
            h_flex()
                .h(px(18.0))
                .px_1p5()
                .items_center()
                .rounded(px(3.0))
                .bg(p.surface.active)
                .text_xs()
                .text_color(p.muted_foreground)
                .child(s)
        }))
        .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
        .child(render_mode_toggle(
            app.clone(),
            tab_id,
            options.side_by_side,
        ))
        .child(render_ignore_menu(
            app.clone(),
            tab_id,
            options.ignore_whitespace,
        ))
        .child(render_highlight_menu(
            app.clone(),
            tab_id,
            options.highlight,
        ))
        .child(render_appearance_menu(app.clone(), tab_id, options))
        .child(render_settings_menu());

    let columns_header = if !diff.loading && diff.error.is_none() {
        tabs::render_diff_columns_header(diff, options.side_by_side, p)
    } else {
        None
    };

    let body = if diff.loading {
        placeholder(rust_i18n::t!("git.diff.loading").to_string(), p)
    } else if let Some(error) = diff.error.clone() {
        placeholder_colored(error, p.danger)
    } else {
        match diff.content.as_ref() {
            None => placeholder(rust_i18n::t!("git.diff.none").to_string(), p),
            Some(content) if content.binary => {
                placeholder(rust_i18n::t!("git.diff.binary").to_string(), p)
            }
            Some(content) => {
                let row_list = rows
                    .unwrap_or_else(|| build_rows(&content.old_text, &content.new_text, &options));
                let row_count = row_list.len();
                let row_list = Rc::new(row_list);
                let row_sizes = Rc::new(vec![size(px(0.0), px(LINE_HEIGHT)); row_count]);
                let scroll = diff.scroll_handle.clone();
                let options_for_rows = options;
                let palette = p.clone();
                let list = v_virtual_list(
                    app.clone(),
                    ElementId::Name("git-diff-virtual-list".into()),
                    row_sizes,
                    move |_workspace, range, _window, _cx| {
                        range
                            .filter_map(|index| {
                                row_list
                                    .get(index)
                                    .cloned()
                                    .map(|row| render_row(row, &options_for_rows, &palette))
                            })
                            .collect()
                    },
                )
                .track_scroll(&scroll)
                .w_full()
                .h_full();
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(content.truncated, |el| {
                        el.child(placeholder(
                            rust_i18n::t!("git.diff.truncated").to_string(),
                            p,
                        ))
                    })
                    .child(ScrollBounce::new("git-diff-scroll", &scroll, list).enabled(false))
                    .child(Scrollbar::vertical(&scroll).viewport_from_layout())
                    .into_any_element()
            }
        }
    };

    v_flex()
        .flex_1()
        .min_h_0()
        .children(tab_bar)
        .child(header)
        .children(columns_header)
        .child(body)
        .into_any_element()
}

pub(crate) fn placeholder(text: String, p: &Palette) -> AnyElement {
    placeholder_colored(text, p.muted_foreground)
}

pub(crate) fn placeholder_colored(text: String, color: Hsla) -> AnyElement {
    div()
        .p_3()
        .text_sm()
        .text_color(color)
        .child(text)
        .into_any_element()
}
