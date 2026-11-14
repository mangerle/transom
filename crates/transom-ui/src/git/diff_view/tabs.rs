//! 差异视图标签栏与双栏列头渲染（对齐 IDEA 差异视图风格）。
//!
//! 设计原理（Why）：
//! - 将标签栏（Tabs）与列头（Header）逻辑独立提取，保持 `diff_view.rs` 紧凑且单文件不超行；
//! - 标签栏遵循「一个标签一个名字」原则，提供文件类型图标、路径名与关闭按钮；
//! - 双栏列头仅对称展示对比基准与当前版本，不再用固定像素伪对齐动态变化的行号槽，
//!   避免行号开关或横向滚动时出现错位。

use std::collections::HashMap;
use std::path::Path;

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::git::actions::DiffTabCloseScope;
use crate::git::types::{GitDiffState, GitWorkspaceState as State};
use crate::palette::Palette;
use crate::theme::tokens::{control, size};
use crate::widgets::{
    IconButton, IconButtonSize, underline_tab_active_line, underline_tab_bar_line,
};
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitRequestId, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 为同批差异标签计算去重后的展示名称：同名不同目录附父目录名，同路径不同比较侧附目标侧标签。
pub(crate) fn disambiguated_tab_titles(tabs: &[GitDiffState]) -> Vec<String> {
    let mut name_counts: HashMap<String, usize> = HashMap::with_capacity(tabs.len());
    let mut path_counts: HashMap<&str, usize> = HashMap::with_capacity(tabs.len());
    for tab in tabs {
        let name = tab_basename(&tab.path);
        *name_counts.entry(name).or_insert(0) += 1;
        *path_counts.entry(tab.path.as_str()).or_insert(0) += 1;
    }

    tabs.iter()
        .map(|tab| {
            let name = tab_basename(&tab.path);
            if path_counts.get(tab.path.as_str()).copied().unwrap_or(0) > 1
                && !tab.right_label.is_empty()
            {
                return format!("{name} ({})", tab.right_label);
            }
            if name_counts.get(&name).copied().unwrap_or(0) > 1
                && let Some(parent) = Path::new(&tab.path)
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .filter(|s| !s.is_empty())
            {
                return format!("{name} ({parent})");
            }
            name
        })
        .collect()
}

fn tab_basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// 渲染顶部差异文件标签栏（支持横向滚动、重名目录消歧、中键关闭与右键批量关闭菜单）。
pub(crate) fn render_diff_tab_bar(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> Option<AnyElement> {
    if state.diff_tabs.is_empty() {
        return None;
    }
    let total_tabs = state.diff_tabs.len();
    let titles = disambiguated_tab_titles(&state.diff_tabs);
    let bar = h_flex()
        .id("git-diff-tab-bar")
        .relative()
        .w_full()
        .h(rems(size::TAB_BAR_HEIGHT))
        .flex_none()
        .items_center()
        .px_1()
        .gap_1()
        .overflow_x_scrollbar()
        .child(underline_tab_bar_line(p))
        .children(state.diff_tabs.iter().zip(titles).enumerate().map(
            |(idx, (tab_item, display_title))| {
                render_single_diff_tab(
                    idx,
                    total_tabs,
                    state.active_diff_tab == Some(idx),
                    tab_item,
                    display_title,
                    app.clone(),
                    tab_id,
                    p,
                )
            },
        ));
    Some(bar.into_any_element())
}

#[allow(clippy::too_many_arguments)]
fn render_single_diff_tab(
    idx: usize,
    total_tabs: usize,
    is_active: bool,
    tab_item: &GitDiffState,
    display_title: String,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> AnyElement {
    let request_id: GitRequestId = tab_item.request_id;
    let tab_app = app.clone();
    let outer_tab_app = app.clone();
    let middle_app = app.clone();
    let close_app = app.clone();
    let menu_app = app;
    let tooltip_text = rust_i18n::t!(
        "git.diff.tab_tooltip",
        path = tab_item.path,
        left = tab_item.left_label,
        right = tab_item.right_label
    )
    .to_string();

    h_flex()
        .id(ElementId::Name(
            format!("git-diff-tab-item-{request_id}").into(),
        ))
        .relative()
        .h_full()
        .flex_none()
        .items_center()
        .gap_1()
        .pl_2()
        .pr_1()
        .cursor_pointer()
        .bg(if is_active {
            p.tab_active
        } else {
            gpui::transparent_black()
        })
        .when(!is_active, |el| el.hover(|s| s.bg(p.list_hover)))
        .when(is_active, |el| el.child(underline_tab_active_line(p)))
        .on_mouse_down(MouseButton::Middle, move |_, _, cx| {
            cx.stop_propagation();
            middle_app.update(cx, |this, cx| {
                this.git_close_diff_tab_by_request(tab_id, request_id, cx);
            });
        })
        .on_click(move |_, _, cx| {
            outer_tab_app.update(cx, |this, cx| {
                this.git_select_diff_tab_by_request(tab_id, request_id, cx);
            });
        })
        .context_menu(move |menu, _window, _cx| {
            let close_self = menu_app.clone();
            let close_others = menu_app.clone();
            let close_left = menu_app.clone();
            let close_right = menu_app.clone();
            let close_all = menu_app.clone();
            menu.item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_tab").to_string()).on_click(
                    move |_, _, cx| {
                        close_self.update(cx, |this, cx| {
                            this.git_close_diff_tab_by_request(tab_id, request_id, cx);
                        });
                    },
                ),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_other_tabs").to_string())
                    .disabled(total_tabs <= 1)
                    .on_click(move |_, _, cx| {
                        close_others.update(cx, |this, cx| {
                            this.git_close_diff_tabs_scope(
                                tab_id,
                                Some(request_id),
                                DiffTabCloseScope::Other,
                                cx,
                            );
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_left_tabs").to_string())
                    .disabled(idx == 0)
                    .on_click(move |_, _, cx| {
                        close_left.update(cx, |this, cx| {
                            this.git_close_diff_tabs_scope(
                                tab_id,
                                Some(request_id),
                                DiffTabCloseScope::Left,
                                cx,
                            );
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_right_tabs").to_string())
                    .disabled(idx + 1 >= total_tabs)
                    .on_click(move |_, _, cx| {
                        close_right.update(cx, |this, cx| {
                            this.git_close_diff_tabs_scope(
                                tab_id,
                                Some(request_id),
                                DiffTabCloseScope::Right,
                                cx,
                            );
                        });
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_all_tabs").to_string()).on_click(
                    move |_, _, cx| {
                        close_all.update(cx, |this, cx| {
                            this.git_close_diff_tabs_scope(
                                tab_id,
                                None,
                                DiffTabCloseScope::All,
                                cx,
                            );
                        });
                    },
                ),
            )
        })
        .child(
            Button::new(ElementId::Name(format!("git-diff-tab-{request_id}").into()))
                .text()
                .xsmall()
                .accessibility_label(display_title.clone())
                .tooltip(tooltip_text)
                .child(
                    AppIconElement::new(AppIcon::for_file(&tab_item.path))
                        .with_rems(rems(control::COMPACT_ICON)),
                )
                .child(
                    div()
                        .text_color(if is_active {
                            p.tab_active_foreground
                        } else {
                            p.tab_foreground
                        })
                        .child(display_title),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    tab_app.update(cx, |this, cx| {
                        this.git_select_diff_tab_by_request(tab_id, request_id, cx);
                    });
                }),
        )
        .child(
            IconButton::new(
                ElementId::Name(format!("git-diff-tab-close-{request_id}").into()),
                UiIcon::CLOSE,
                "Close diff tab",
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                close_app.update(cx, |this, cx| {
                    this.git_close_diff_tab_by_request(tab_id, request_id, cx);
                });
            }),
        )
        .into_any_element()
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
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .text_xs()
        .child(
            h_flex().w_1_2().h_full().px_2().items_center().child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap_1p5()
                    .items_center()
                    .overflow_hidden()
                    .child(
                        AppIconElement::new(UiIcon::VCS)
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
                            .truncate()
                            .text_color(p.muted_foreground.opacity(0.75))
                            .child(short_path),
                    ),
            ),
        )
        .child(
            h_flex()
                .w_1_2()
                .h_full()
                .px_2()
                .items_center()
                .border_color(p.divider(p.border))
                .border_l_1()
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
