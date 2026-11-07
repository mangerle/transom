//! Git 推送弹窗子视图组件（待推送提交列表、提交详情卡片与文件变动树）。
//!
//! 设计原理（Why）：
//! - 将列表行与卡片组件拆分为独立子视图，保持单一文件生产代码在 500 行以内；
//! - 遵循 openidea / IntelliJ IDEA 规范，双栏右侧提供完整提交说明卡片与改动文件列表；
//! - 文件状态按语义色（A 绿、D 红、M 棕/黄、R 蓝）清晰区分，提升可读性。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ChangeCode, CommitEntry, CommitFile};

/// 渲染左栏单条待推送提交。
pub(super) fn render_commit_row(
    commit: &CommitEntry,
    is_selected: bool,
    app: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let oid_clone = commit.oid.clone();
    let app_select = app.clone();
    let item_id = ElementId::Name(format!("push-commit-{}", commit.oid).into());

    h_flex()
        .id(item_id)
        .w_full()
        .px_2p5()
        .py_2()
        .gap_2()
        .items_center()
        .cursor_pointer()
        .bg(if is_selected {
            p.primary.opacity(0.18)
        } else {
            p.background
        })
        .hover(|style| {
            if !is_selected {
                style.bg(p.muted.opacity(0.35))
            } else {
                style
            }
        })
        .child(
            Icon::new(IconName::GitCommitHorizontal)
                .xsmall()
                .text_color(if is_selected {
                    p.primary
                } else {
                    p.muted_foreground
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_xs()
                        .font_weight(if is_selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(p.foreground)
                        .truncate()
                        .child(commit.subject.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(
                            div()
                                .truncate()
                                .max_w(px(140.0))
                                .child(commit.author_name.clone()),
                        )
                        .child(
                            div()
                                .px_1()
                                .bg(p.muted.opacity(0.3))
                                .rounded_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(commit.short_oid().to_string()),
                        ),
                ),
        )
        .on_click(move |_, _, cx| {
            let oid = oid_clone.clone();
            app_select.update(cx, |this, cx| {
                this.git_select_push_commit(oid, cx);
            });
        })
        .into_any_element()
}

/// 渲染右栏顶部选中的提交详情卡片（完整 Message 与作者信息）。
pub(super) fn render_commit_detail_card(
    commit: &CommitEntry,
    app: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let oid_full = commit.oid.clone();
    let oid_short = commit.short_oid().to_string();
    let app_copy = app.clone();

    let full_message = if commit.body.is_empty() {
        commit.subject.clone()
    } else {
        format!("{}\n\n{}", commit.subject, commit.body)
    };

    v_flex()
        .w_full()
        .p_2p5()
        .gap_2()
        .bg(p.muted.opacity(0.18))
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.foreground)
                        .child(format!("{} <{}>", commit.author_name, commit.author_email)),
                )
                .child(div().flex_1())
                .child(
                    h_flex()
                        .id(ElementId::Name(
                            format!("copy-push-oid-{}", commit.oid).into(),
                        ))
                        .gap_1()
                        .items_center()
                        .cursor_pointer()
                        .px_1p5()
                        .py_0p5()
                        .bg(p.muted.opacity(0.3))
                        .rounded_sm()
                        .hover(|s| s.bg(p.muted.opacity(0.6)))
                        .child(
                            Icon::new(IconName::Copy)
                                .xsmall()
                                .text_color(p.muted_foreground),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(oid_short),
                        )
                        .on_click(move |_, _, cx| {
                            let oid = oid_full.clone();
                            app_copy.update(cx, |this, cx| {
                                this.write_clipboard(&oid, cx);
                            });
                        }),
                ),
        )
        .child(
            div()
                .w_full()
                .max_h(px(64.0))
                .overflow_y_scrollbar()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(full_message),
        )
        .into_any_element()
}

/// 渲染右栏单个变动文件行。
pub(super) fn render_file_row(
    file: &CommitFile,
    tab_id: Option<transom_core::TabId>,
    app: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let change = file.change;
    let (color, label) = match change {
        ChangeCode::Added => (p.success, "A"),
        ChangeCode::Deleted => (p.danger, "D"),
        ChangeCode::Renamed => (p.primary, "R"),
        ChangeCode::Copied => (p.primary, "C"),
        ChangeCode::TypeChange => (p.warning, "T"),
        _ => (p.warning, "M"),
    };

    let file_path = file.path.clone();
    let app_diff = app.clone();
    let row_id = ElementId::Name(format!("push-file-{}", file.path).into());

    let (file_name, parent_dir) = match file.path.rsplit_once('/') {
        Some((parent, name)) => (name, parent),
        None => (file.path.as_str(), ""),
    };

    h_flex()
        .id(row_id)
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .cursor_pointer()
        .hover(|s| s.bg(p.muted.opacity(0.3)))
        .child(
            div()
                .w(px(14.0))
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.foreground)
                .child(file_name.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(p.muted_foreground.opacity(0.75))
                .child(parent_dir.to_string()),
        )
        .on_click(move |_, _, cx| {
            if let Some(t_id) = tab_id {
                let p_str = file_path.clone();
                app_diff.update(cx, |this, cx| {
                    this.git_open_diff_modal(t_id, &p_str, cx);
                });
            }
        })
        .into_any_element()
}
