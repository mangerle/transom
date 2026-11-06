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

use menus::{
    render_appearance_menu, render_highlight_menu, render_ignore_menu, render_mode_toggle,
    render_settings_menu,
};
use rows::{build_rows, render_row};

use crate::app::AppShell;
use crate::git::GitWorkspaceState as State;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

/// 行高与行号列宽（px）。
pub(crate) const LINE_HEIGHT: f32 = 18.0;

pub(crate) const GUTTER_WIDTH: f32 = 44.0;

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
    let Some(diff) = state.diff.as_ref() else {
        return placeholder(rust_i18n::t!("git.diff.none").to_string(), p);
    };
    let options = state.diff_options;
    let header = h_flex()
        .id("git-diff-header")
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .text_xs()
        .flex_none()
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(
            Icon::new(IconName::GitCompare)
                .xsmall()
                .text_color(p.primary),
        )
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_color(p.foreground)
                .child(diff.path.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(p.muted_foreground)
                .child(format!("{} → {}", diff.left_label, diff.right_label)),
        )
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
                let rows = build_rows(&content.old_text, &content.new_text, &options);
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(content.truncated, |el| {
                        el.child(placeholder(
                            rust_i18n::t!("git.diff.truncated").to_string(),
                            p,
                        ))
                    })
                    .children(rows.into_iter().map(|row| render_row(row, &options, p)))
                    .into_any_element()
            }
        }
    };
    v_flex()
        .flex_1()
        .min_h_0()
        .child(header)
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
