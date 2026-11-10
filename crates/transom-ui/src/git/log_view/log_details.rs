//! Git 日志右侧栏：选中的提交变更文件清单与提交详情。
//!
//! 设计原理（Why）：
//! - 对齐 IntelliJ IDEA / openIDEA 标准 Git 日志右侧三栏布局；
//! - 上半部分为变更文件清单（树/列表），支持单选、双击呼出独立全功能 Diff 对比弹窗，
//!   并在顶部工具栏提供「在标签页打开」和「差异对比」动作；
//! - 下半部分为提交说明（标题、换行正文、Hash、作者与邮箱、所属分支）；
//! - 上下两部分由垂直拖拽切分器（v_resizable）组织，用户可按需自由调整高低比例。

use super::files_tree::{FileTreeNode, render_tree_nodes};
use super::placeholder;
use crate::app::AppShell;
use crate::divider::v_resizable;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

type State = crate::git::GitWorkspaceState;

/// 渲染右侧面板（变更文件树 + 提交说明）。
pub(crate) fn render_right_panel(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    width: f32,
    p: &Palette,
) -> AnyElement {
    let split = state.right_split.clone();
    let files_part = render_files_section(state, app.clone(), tab_id, p);
    let details_part = render_details_section(state, p);

    v_flex()
        .w(px(width))
        .flex_none()
        .min_h_0()
        .h_full()
        .overflow_hidden()
        .border_color(p.divider(p.border))
        .border_l_1()
        .child(
            v_resizable(ElementId::Name(format!("git-right-split-{tab_id}").into()))
                .with_state(&split)
                .child(
                    resizable_panel()
                        .size(px(240.0))
                        .size_range(px(120.0)..px(600.0))
                        .child(files_part),
                )
                .child(
                    resizable_panel()
                        .size(px(220.0))
                        .size_range(px(100.0)..px(500.0))
                        .child(details_part),
                ),
        )
        .into_any_element()
}

/// 上半部分：变更文件清单与操作条。
fn render_files_section(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> AnyElement {
    let file_count = state.commit_files.len();
    let count_text = rust_i18n::t!("git.files.count", count = file_count).to_string();

    let rows: Vec<AnyElement> = if state.commit_files_loading {
        vec![placeholder(rust_i18n::t!("git.log.loading").to_string(), p)]
    } else if state.selected_commit.is_none() {
        vec![placeholder(
            rust_i18n::t!("git.files.need_commit").to_string(),
            p,
        )]
    } else if state.commit_files.is_empty() {
        vec![placeholder(rust_i18n::t!("git.files.empty").to_string(), p)]
    } else {
        // 构建 IDEA 风格层级目录树并自动压缩单子目录
        let active_path = state.active_diff().map(|d| d.path.as_str());
        let tree = FileTreeNode::build_tree(&state.commit_files);
        render_tree_nodes(&tree, 0, active_path, &app, &state.project, tab_id, p)
    };

    let toolbar_app = app.clone();
    let first_file_path = state.commit_files.first().map(|f| f.path.clone());

    let toolbar = h_flex()
        .w_full()
        .h(px(24.0))
        .flex_none()
        .px_2()
        .items_center()
        .justify_between()
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_foreground)
                .child(if state.selected_commit.is_some() {
                    count_text
                } else {
                    rust_i18n::t!("git.files.title").to_string()
                }),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    Button::new("git-diff-compare-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::GitCompare)
                        .accessibility_label(rust_i18n::t!("git.details.compare_diff").to_string())
                        .tooltip(rust_i18n::t!("git.details.compare_diff").to_string())
                        .on_click({
                            let toolbar_app = toolbar_app.clone();
                            let first_path = first_file_path.clone();
                            move |_, _, cx| {
                                if let Some(path) = &first_path {
                                    let path = path.clone();
                                    toolbar_app.update(cx, |this, cx| {
                                        this.git_open_diff_modal(tab_id, &path, cx);
                                    });
                                }
                            }
                        }),
                )
                .child(
                    Button::new("git-diff-tab-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ExternalLink)
                        .accessibility_label(rust_i18n::t!("git.details.open_in_tab").to_string())
                        .tooltip(rust_i18n::t!("git.details.open_in_tab").to_string())
                        .on_click({
                            let toolbar_app = toolbar_app.clone();
                            let first_path = first_file_path.clone();
                            move |_, _, cx| {
                                if let Some(path) = &first_path {
                                    let path = path.clone();
                                    toolbar_app.update(cx, |this, cx| {
                                        this.git_diff_commit_file(tab_id, &path, cx);
                                    });
                                }
                            }
                        }),
                ),
        );

    v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .child(toolbar)
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .children(rows),
        )
        .into_any_element()
}

/// 下半部分：提交说明与详细元数据。
fn render_details_section(state: &State, p: &Palette) -> AnyElement {
    let Some(entry) = state.selected_entry() else {
        return v_flex()
            .w_full()
            .h_full()
            .min_h_0()
            .border_color(p.divider(p.border))
            .border_t_1()
            .child(placeholder(
                rust_i18n::t!("git.files.need_commit").to_string(),
                p,
            ))
            .into_any_element();
    };

    let author_text = format!(
        "{} {} <{}>",
        rust_i18n::t!("git.details.author_prefix"),
        entry.author_name,
        entry.author_email
    );
    let time_text = format_time(entry.author_time);

    // 获取所在分支
    let branches: Vec<String> = state
        .branches
        .iter()
        .filter(|b| !b.remote)
        .map(|b| b.name.clone())
        .collect();
    let branch_str = if branches.is_empty() {
        "HEAD".to_string()
    } else {
        branches.join(", ")
    };
    let branches_text = rust_i18n::t!(
        "git.details.branches",
        count = branches.len().max(1),
        branches = branch_str.as_str()
    )
    .to_string();

    v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .border_color(p.divider(p.border))
        .border_t_1()
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_3()
                .gap_2()
                // 提交标题
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.foreground)
                        .child(entry.subject.clone()),
                )
                // 提交正文（如果存在）
                .when(!entry.body.is_empty(), |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(entry.body.clone()),
                    )
                }),
        )
        // 底部元数据栏（自然融入背景，无突兀灰底）
        .child(
            v_flex()
                .flex_none()
                .px_3()
                .py_2()
                .gap_1()
                .border_color(p.divider(p.border))
                .border_t_1()
                .text_xs()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_color(p.muted_foreground)
                        .child(
                            div()
                                .font_family("monospace")
                                .text_color(p.primary)
                                .child(entry.short_oid().to_string()),
                        )
                        .child(div().child(author_text))
                        .child(div().child(time_text)),
                )
                .child(
                    div()
                        .text_color(p.muted_foreground.opacity(0.75))
                        .child(branches_text),
                ),
        )
        .into_any_element()
}

/// 格式化时间戳。
fn format_time(timestamp: i64) -> String {
    // 简易时间格式化为 本地可读 YYYY-MM-DD HH:MM
    let dt = chrono::DateTime::from_timestamp(timestamp, 0);
    match dt {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => "-".to_string(),
    }
}
