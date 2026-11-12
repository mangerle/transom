//! Git 差异视图：标签栏 + 统一差异工具条 + 基于 `gpui_kit::component::diff::Diff` 的渲染主体。
//!
//! 设计原理（Why）：
//! - 与 SFTP 远程编辑的保存前预览共用 `diff_pane` 公共底座，彻底淘汰每帧在主线程
//!   重算 Myers 的手绘行列表；
//! - 差异比对与补丁解析由后台线程计算并缓存在 `GitDiffState::diff_state` 中，
//!   工具条上的上/下一处差异、折叠/展开上下文、并排/统一、空白忽略、高亮粒度、
//!   行号与软换行全部直接驱动公共控件，同时支持语法高亮与划词复制。

mod menus;
mod tabs;

use menus::render_settings_menu;

use crate::app::AppShell;
use crate::diff_pane::{
    OptionsUpdater, render_change_nav_buttons, render_diff_body, render_option_controls,
};
use crate::git::types::GitWorkspaceState as State;
use crate::palette::Palette;
use gpui_kit::base::Disableable;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use std::rc::Rc;
use transom_core::TabId;
use transom_ui_icons::UiIcon;

/// 渲染 Git 差异面板。
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
    let opt_app = app.clone();
    let on_update: OptionsUpdater = Rc::new(move |updater, _window, cx| {
        opt_app.update(cx, |this, cx| {
            this.git_update_diff_options(tab_id, updater, cx);
        });
    });

    let has_changes = diff.diff_state.is_some();
    let diff_summary = if !diff.loading
        && !diff.computing
        && diff.error.is_none()
        && diff.content.as_ref().is_some_and(|c| !c.binary)
    {
        Some(
            rust_i18n::t!(
                "git.diff.differences_count",
                count = diff.summary_hunks
            )
            .to_string(),
        )
    } else {
        None
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
        .child(render_change_nav_buttons(
            "git-diff",
            diff.diff_state.clone(),
            has_changes,
        ))
        .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
        .child(
            Button::new("git-diff-prev-file")
                .ghost()
                .xsmall()
                .icon(UiIcon::ARROW_LEFT)
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
                .icon(UiIcon::ARROW_RIGHT)
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
        .child(render_option_controls("git-diff", options, on_update))
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
    } else if diff.computing {
        placeholder(rust_i18n::t!("git.diff.computing").to_string(), p)
    } else {
        match diff.content.as_ref() {
            None => placeholder(rust_i18n::t!("git.diff.none").to_string(), p),
            Some(content) if content.binary => {
                placeholder(rust_i18n::t!("git.diff.binary").to_string(), p)
            }
            Some(content) => match diff.diff_state.as_ref() {
                None => placeholder(rust_i18n::t!("git.diff.no_changes").to_string(), p),
                Some(diff_state) => {
                    let truncated_banner = content.truncated.then(|| {
                        div()
                            .w_full()
                            .px_3()
                            .py_1()
                            .bg(p.tone.warning_subtle)
                            .border_color(p.divider(p.border))
                            .border_b_1()
                            .text_xs()
                            .text_color(p.warning)
                            .child(rust_i18n::t!("git.diff.truncated").to_string())
                            .into_any_element()
                    });
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .children(truncated_banner)
                        .child(
                            render_diff_body(diff_state, options)
                                .flex_1()
                                .w_full(),
                        )
                        .into_any_element()
                }
            },
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
