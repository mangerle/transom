//! 差异视图标签栏与双栏列头渲染（对齐 IDEA 差异视图风格）。
//!
//! 设计原理（Why）：
//! - 将标签栏（Tabs）与列头（Header）逻辑独立提取，保持 `diff_view.rs` 紧凑且单文件不超行；
//! - 标签栏遵循「一个标签一个名字」原则，提供文件类型图标、路径名与关闭按钮；
//! - 双栏列头与下方代码行的 `side`（各占 50% 宽度并留出行号槽）严格对齐，标明对比基准与当前版本。

use std::path::Path;

use super::GUTTER_WIDTH;
use crate::app::AppShell;
use crate::git::types::{GitDiffState, GitWorkspaceState as State};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::scroll::ScrollableElement;

use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::TabId;

/// 渲染顶部差异文件标签栏（一个标签一个名字）。
pub(crate) fn render_diff_tab_bar(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> Option<AnyElement> {
    if state.diff_tabs.is_empty() {
        return None;
    }
    let bar = h_flex()
        .id("git-diff-tab-bar")
        .w_full()
        .h(px(28.0))
        .bg(p.muted)
        .border_color(p.divider(p.border))
        .border_b_1()
        .items_center()
        .px_1()
        .gap_1()
        .overflow_x_scrollbar()
        .children(state.diff_tabs.iter().enumerate().map(|(idx, tab_item)| {
            let is_active = state.active_diff_tab == Some(idx);
            let file_name = Path::new(&tab_item.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| tab_item.path.clone());
            let tab_app = app.clone();
            let close_app = app.clone();
            h_flex()
                .id(ElementId::NamedInteger("git-diff-tab".into(), idx as u64))
                .items_center()
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded(px(3.0))
                .cursor_pointer()
                .bg(if is_active {
                    p.background
                } else {
                    gpui::transparent_black()
                })
                .text_color(if is_active {
                    p.foreground
                } else {
                    p.muted_foreground
                })
                .border_color(if is_active {
                    p.primary
                } else {
                    gpui::transparent_black()
                })
                .border_b_2()
                .hover(|s| if !is_active { s.bg(p.muted) } else { s })
                .child(
                    Icon::new(IconName::FileCode)
                        .xsmall()
                        .text_color(if is_active {
                            p.primary
                        } else {
                            p.muted_foreground
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(if is_active {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(file_name),
                )
                .child(
                    h_flex()
                        .id(ElementId::NamedInteger(
                            "git-diff-tab-close".into(),
                            idx as u64,
                        ))
                        .p_0p5()
                        .rounded(px(2.0))
                        .hover(|s| s.bg(p.muted))
                        .child(
                            Icon::new(IconName::X)
                                .xsmall()
                                .text_color(p.muted_foreground),
                        )
                        .on_click(move |_, _, cx| {
                            close_app.update(cx, |this, cx| {
                                this.git_close_diff_tab(tab_id, idx, cx);
                            });
                        }),
                )
                .on_click(move |_, _, cx| {
                    tab_app.update(cx, |this, cx| {
                        this.git_select_diff_tab(tab_id, idx, cx);
                    });
                })
        }));
    Some(bar.into_any_element())
}

/// 渲染双栏对比列头：左侧基准版本与文件，右侧当前版本。
pub(crate) fn render_diff_columns_header(
    diff: &GitDiffState,
    side_by_side: bool,
    p: &Palette,
) -> Option<AnyElement> {
    if !side_by_side {
        return None;
    }
    let left_title = format!("{} {}", diff.left_label, diff.path);
    let right_title = if diff.right_label.is_empty() {
        rust_i18n::t!("git.diff.current_version", path = diff.path.as_str()).to_string()
    } else {
        format!("{} {}", diff.right_label, diff.path)
    };

    let header = h_flex()
        .id("git-diff-columns-header")
        .w_full()
        .h(px(24.0))
        .bg(p.muted)
        .border_color(p.divider(p.border))
        .border_b_1()
        .text_xs()
        .child(
            h_flex()
                .w_1_2()
                .h_full()
                .items_center()
                .child(div().w(px(GUTTER_WIDTH)).flex_none())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(p.muted_foreground)
                        .child(left_title),
                ),
        )
        .child(
            h_flex()
                .w_1_2()
                .h_full()
                .items_center()
                .border_color(p.divider(p.border))
                .border_l_1()
                .child(div().w(px(GUTTER_WIDTH)).flex_none())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(p.muted_foreground)
                        .child(right_title),
                ),
        );
    Some(header.into_any_element())
}
