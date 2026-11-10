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
use crate::widgets::{IconButton, IconButtonSize};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
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
        .h(px(26.0))
        .flex_none()
        .border_color(p.divider(p.border))
        .border_b_1()
        .items_center()
        .px_1()
        .gap_1()
        .overflow_hidden()
        .children(state.diff_tabs.iter().enumerate().map(|(idx, tab_item)| {
            let is_active = state.active_diff_tab == Some(idx);
            let file_name = Path::new(&tab_item.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| tab_item.path.clone());
            let tab_app = app.clone();
            let close_app = app.clone();
            h_flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded(px(2.0))
                .bg(if is_active {
                    p.background
                } else {
                    gpui::transparent_black()
                })
                .border_color(if is_active {
                    p.primary
                } else {
                    gpui::transparent_black()
                })
                .border_b_2()
                .child(
                    Button::new(ElementId::Name(
                        format!("git-diff-tab-{}", tab_item.path).into(),
                    ))
                    .ghost()
                    .xsmall()
                    .selected(is_active)
                    .icon(IconName::FileCode)
                    .label(file_name)
                    .on_click(move |_, _, cx| {
                        tab_app.update(cx, |this, cx| {
                            this.git_select_diff_tab(tab_id, idx, cx);
                        });
                    }),
                )
                .child(
                    IconButton::new(
                        ElementId::Name(format!("git-diff-tab-close-{}", tab_item.path).into()),
                        IconName::X,
                        "Close diff tab",
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        close_app.update(cx, |this, cx| {
                            this.git_close_diff_tab(tab_id, idx, cx);
                        });
                    }),
                )
        }));
    Some(bar.into_any_element())
}

/// 渲染双栏对比列头：左侧基准版本，右侧当前版本（对齐 IDEA 样式，去除长路径重复截断）。
pub(crate) fn render_diff_columns_header(
    diff: &GitDiffState,
    side_by_side: bool,
    p: &Palette,
) -> Option<AnyElement> {
    if !side_by_side {
        return None;
    }
    let right_label = if diff.right_label.is_empty() {
        rust_i18n::t!("git.diff.current_version_label").to_string()
    } else {
        diff.right_label.clone()
    };

    let short_path = Path::new(&diff.path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| diff.path.clone());

    let header = h_flex()
        .id("git-diff-columns-header")
        .w_full()
        .h(px(22.0))
        .flex_none()
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
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .gap_1p5()
                        .items_center()
                        .overflow_hidden()
                        .child(
                            Icon::new(IconName::GitCommitHorizontal)
                                .xsmall()
                                .text_color(p.primary),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .child(diff.left_label.clone()),
                        )
                        .child(
                            div()
                                .text_color(p.muted_foreground.opacity(0.75))
                                .child(short_path),
                        ),
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
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .child(right_label),
                        ),
                ),
        );
    Some(header.into_any_element())
}
