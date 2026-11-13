//! Git 差异视图：标签栏 + 统一差异工具条 + 基于 `gpui_kit::component::diff::Diff` 的渲染主体。
//!
//! 设计原理（Why）：
//! - 与 SFTP 远程编辑的保存前预览共用 `diff_pane` 公共底座，彻底淘汰每帧在主线程
//!   重算 Myers 的手绘行列表；
//! - 差异比对与补丁解析由后台线程计算并缓存在 `GitDiffState::diff_state` 中，
//!   工具条上的上/下一处差异、折叠/展开上下文、并排/统一、空白忽略、高亮粒度、
//!   行号与软换行全部直接驱动公共控件，同时支持语法高亮与划词复制。

mod hunk_gutter;
mod menus;
mod tabs;

use hunk_gutter::attach_hunk_actions;
use menus::{DiffMoreMenuParams, render_settings_menu};

use crate::app::AppShell;
use crate::diff_pane::{
    DiffSearchBarProps, OptionsUpdater, render_change_nav_with_handlers, render_diff_body,
    render_diff_search_bar, render_option_controls,
};
use crate::git::WORKSPACE_HEADER_HEIGHT;
use crate::git::types::GitWorkspaceState as State;
use crate::keymap::{DIFF_VIEW_CONTEXT, DiffFindInView, DiffNextChange, DiffPrevChange};
use crate::palette::Palette;
use gpui_kit::base::Disableable;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
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
    let prev_change_app = app.clone();
    let next_change_app = app.clone();
    let refresh_app = app.clone();
    let open_app = app.clone();
    let search_toggle_app = app.clone();
    let search_prev_app = app.clone();
    let search_next_app = app.clone();
    let search_close_app = app.clone();
    let kbd_prev_app = app.clone();
    let kbd_next_app = app.clone();
    let kbd_find_app = app.clone();
    let on_update: OptionsUpdater = Rc::new(move |updater, _window, cx| {
        opt_app.update(cx, |this, cx| {
            this.git_update_diff_options(tab_id, updater, cx);
        });
    });

    let has_changes = diff.diff_state.is_some();
    let nav_disabled = total_tabs <= 1 && !has_changes;
    let show_summary = !diff.loading
        && !diff.computing
        && diff.error.is_none()
        && diff.content.as_ref().is_some_and(|c| !c.binary);
    let diff_summary_pill = show_summary.then(|| {
        render_diff_summary_pill(
            diff.whitespace_only,
            diff.summary_hunks,
            diff.summary_additions,
            diff.summary_deletions,
            p,
        )
    });

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
        .h(rems(WORKSPACE_HEADER_HEIGHT))
        .flex_none()
        .px_2()
        .gap_1p5()
        .items_center()
        .text_xs()
        .bg(p.background)
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(render_change_nav_with_handlers(
            "git-diff",
            nav_disabled,
            Rc::new(move |_, cx| {
                prev_change_app.update(cx, |this, cx| {
                    this.git_step_diff_change(tab_id, false, cx);
                });
            }),
            Rc::new(move |_, cx| {
                next_change_app.update(cx, |this, cx| {
                    this.git_step_diff_change(tab_id, true, cx);
                });
            }),
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
        .children(diff_summary_pill)
        .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
        .child(render_option_controls("git-diff", options, on_update))
        .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
        .child(
            Button::new("git-diff-search-toggle")
                .ghost()
                .xsmall()
                .icon(UiIcon::SEARCH)
                .when(state.diff_search.open, |btn| btn.secondary())
                .accessibility_label(rust_i18n::t!("git.diff.search_toggle").to_string())
                .tooltip(rust_i18n::t!("git.diff.search_toggle").to_string())
                .on_click(move |_, window, cx| {
                    search_toggle_app.update(cx, |this, cx| {
                        this.git_toggle_diff_search(tab_id, window, cx);
                    });
                }),
        )
        .child(
            Button::new("git-diff-refresh")
                .ghost()
                .xsmall()
                .icon(UiIcon::REFRESH)
                .accessibility_label(rust_i18n::t!("git.diff.refresh").to_string())
                .tooltip(rust_i18n::t!("git.diff.refresh").to_string())
                .on_click(move |_, _, cx| {
                    refresh_app.update(cx, |this, cx| {
                        this.git_refresh_active_diff(tab_id, cx);
                    });
                }),
        )
        .child(
            Button::new("git-diff-open-source")
                .ghost()
                .xsmall()
                .icon(UiIcon::EDIT)
                .accessibility_label(rust_i18n::t!("git.diff.open_source").to_string())
                .tooltip(rust_i18n::t!("git.diff.open_source").to_string())
                .on_click(move |_, _, cx| {
                    open_app.update(cx, |this, cx| {
                        this.git_open_diff_source(tab_id, cx);
                    });
                }),
        )
        .child(render_settings_menu(DiffMoreMenuParams {
            app: app.clone(),
            tab_id,
            active_request_id: diff.request_id,
            total_tabs,
            collapse_unchanged: options.collapse_unchanged,
        }));

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
                None => {
                    let message_key = if diff.whitespace_only {
                        "git.diff.whitespace_only"
                    } else {
                        "git.diff.no_changes"
                    };
                    placeholder(rust_i18n::t!(message_key).to_string(), p)
                }
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
                    let diff_widget = attach_hunk_actions(
                        render_diff_body(diff_state, options)
                            .decorations(state.diff_search.build_decorations(p)),
                        diff,
                        state.status.as_ref(),
                        app.clone(),
                        tab_id,
                    );
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .children(truncated_banner)
                        .child(diff_widget.flex_1().w_full())
                        .into_any_element()
                }
            },
        }
    };

    let search_bar = render_diff_search_bar(DiffSearchBarProps {
        id_prefix: "git-diff",
        state: &state.diff_search,
        p,
        on_prev: Rc::new(move |_, cx| {
            search_prev_app.update(cx, |this, cx| {
                this.git_step_diff_search(tab_id, false, cx);
            });
        }),
        on_next: Rc::new(move |_, cx| {
            search_next_app.update(cx, |this, cx| {
                this.git_step_diff_search(tab_id, true, cx);
            });
        }),
        on_close: Rc::new(move |_, cx| {
            search_close_app.update(cx, |this, cx| {
                this.git_close_diff_search(tab_id, cx);
            });
        }),
    });

    v_flex()
        .id("git-diff-view-pane")
        .key_context(DIFF_VIEW_CONTEXT)
        .on_action(move |_: &DiffPrevChange, _, cx| {
            kbd_prev_app.update(cx, |this, cx| {
                this.git_step_diff_change(tab_id, false, cx);
            });
        })
        .on_action(move |_: &DiffNextChange, _, cx| {
            kbd_next_app.update(cx, |this, cx| {
                this.git_step_diff_change(tab_id, true, cx);
            });
        })
        .on_action(move |_: &DiffFindInView, window, cx| {
            kbd_find_app.update(cx, |this, cx| {
                this.git_open_diff_search(tab_id, window, cx);
            });
        })
        .flex_1()
        .min_h_0()
        .children(tab_bar)
        .child(header)
        .children(search_bar)
        .children(columns_header)
        .child(body)
        .into_any_element()
}

/// 渲染工具栏右侧的差异统计徽章（块数 + 新增/删除行数，或空白忽略提示）。
fn render_diff_summary_pill(
    whitespace_only: bool,
    hunks: usize,
    additions: usize,
    deletions: usize,
    p: &Palette,
) -> AnyElement {
    let base = h_flex()
        .h(px(18.0))
        .px_1p5()
        .gap_1p5()
        .items_center()
        .rounded(px(3.0))
        .bg(p.surface.active)
        .text_xs();

    if whitespace_only {
        return base
            .text_color(p.warning)
            .child(rust_i18n::t!("git.diff.whitespace_only").to_string())
            .into_any_element();
    }

    let mut pill = base
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("git.diff.differences_count", count = hunks).to_string());
    if additions > 0 || deletions > 0 {
        pill = pill
            .child(div().text_color(p.success).child(format!("+{additions}")))
            .child(div().text_color(p.danger).child(format!("-{deletions}")));
    }
    pill.into_any_element()
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
