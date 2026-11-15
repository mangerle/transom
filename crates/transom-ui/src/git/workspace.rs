//! Git 标签页主体：顶部工具栏 + 日志工作区 / 概览。
//!
//! 设计原理（Why）：
//! - 布局照 IDEA 的 Git 工具窗口：左引用栏、中提交表（含提交图与文件清单）、
//!   右详情与差异三栏。用户从 IDE 过来时不需要重新学一遍信息在哪；
//! - 顶部固定两个入口——「提交」开模态框，「操作」在概览与日志工作区之间切换；
//!   刷新 / 抓取 / 拉取 / 推送作为工具栏动作，与 IDEA 的工具条一致；
//! - 渲染期只 `read` 标签页的 `Entity<GitWorkspace>` 状态，任何变更都经实体
//!   `update` 或 `GitWorkspaceEvent` 回调：这样既避开借用冲突，也让"点击后改什么"
//!   集中在一处。

use super::GitWorkspace;
use super::log_view;
use super::types::{GitBadge, GitView};
use crate::divider::{h_resizable, v_resizable};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染一个 Git 标签页。
pub(crate) fn render_git_tab(
    workspace: Entity<GitWorkspace>,
    badge: GitBadge,
    window: &mut Window,
    cx: &mut App,
    p: &Palette,
) -> AnyElement {
    workspace.update(cx, |ws, cx| ws.clear_pending_commit_message(window, cx));
    let (tab_id, view, bottom_log_open, bottom_log_split) = {
        let ws = workspace.read(cx);
        let state = &ws.state;
        (
            ws.tab_id,
            state.view,
            state.bottom_log_open,
            state.bottom_log_split.clone(),
        )
    };
    let body = match view {
        GitView::Log => render_git_log_view(workspace.clone(), p, cx),
        GitView::Overview => {
            let commit_page = super::commit_page::render_git_commit_page(workspace.clone(), p, cx);
            if bottom_log_open {
                let bottom_panel = super::bottom_log::render_git_bottom_log_panel(
                    workspace.clone(),
                    badge.branch.clone(),
                    p,
                    cx,
                );
                div()
                    .flex_1()
                    .min_h_0()
                    .size_full()
                    .child(
                        v_resizable(ElementId::Name(
                            format!("git-bottom-log-split-{tab_id}").into(),
                        ))
                        .with_state(&bottom_log_split)
                        .child(resizable_panel().child(commit_page))
                        .child(
                            resizable_panel()
                                .size(px(320.0))
                                .size_range(px(180.0)..px(600.0))
                                .fixed()
                                .child(bottom_panel),
                        ),
                    )
                    .into_any_element()
            } else {
                commit_page
            }
        }
    };
    let mut root = v_flex().size_full().bg(p.background).relative();
    // 提交页自带页签与工具条；只有日志页还用这条顶部工具条。
    if matches!(view, GitView::Log) {
        root = root.child(render_git_toolbar(workspace, badge, p));
    }
    root.child(body).into_any_element()
}

/// 顶部工具条（日志页）：分支角标 + 远程动作。
fn render_git_toolbar(workspace: Entity<GitWorkspace>, badge: GitBadge, p: &Palette) -> AnyElement {
    h_flex()
        .id("git-toolbar")
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .flex_none()
        .border_color(p.divider(p.border))
        .border_b_1()
        .bg(p.background)
        .child(div().flex_1())
        .child(render_git_branch_chip(badge, p))
        .child(render_git_remote_actions(workspace))
        .into_any_element()
}

/// 远程动作按钮组：提交页与日志页共用。
fn render_git_remote_actions(workspace: Entity<GitWorkspace>) -> AnyElement {
    let ws_refresh = workspace.clone();
    let ws_fetch = workspace.clone();
    let ws_pull = workspace.clone();
    let ws_push = workspace;
    h_flex()
        .gap_1()
        .items_center()
        .child(
            Button::new("git-toolbar-refresh")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::REFRESH))
                .accessibility_label(rust_i18n::t!("git.menu.refresh").to_string())
                .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                .on_click(move |_, _, cx| {
                    ws_refresh.update(cx, |ws, cx| {
                        ws.refresh();
                        ws.load_log(0);
                        cx.notify();
                    });
                }),
        )
        .child(
            Button::new("git-toolbar-fetch")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::FETCH))
                .accessibility_label(rust_i18n::t!("git.toolbar.fetch").to_string())
                .tooltip(rust_i18n::t!("git.toolbar.fetch").to_string())
                .on_click(move |_, _, cx| {
                    ws_fetch.update(cx, |ws, cx| ws.fetch(cx));
                }),
        )
        .child(
            Button::new("git-toolbar-pull")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::UPDATE))
                .accessibility_label(rust_i18n::t!("git.toolbar.pull").to_string())
                .tooltip(rust_i18n::t!("git.toolbar.pull").to_string())
                .on_click(move |_, _, cx| {
                    ws_pull.update(cx, |ws, cx| ws.pull(false, cx));
                }),
        )
        .child(
            Button::new("git-toolbar-push")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::PUSH))
                .accessibility_label(rust_i18n::t!("git.toolbar.push").to_string())
                .tooltip(rust_i18n::t!("git.toolbar.push").to_string())
                .on_click(move |_, _, cx| {
                    ws_push.update(cx, |ws, cx| ws.open_push_modal(cx));
                }),
        )
        .into_any_element()
}

/// 当前分支 + 领先/落后角标。
fn render_git_branch_chip(badge: GitBadge, p: &Palette) -> AnyElement {
    h_flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded(px(3.0))
        .bg(p.muted)
        .child(
            AppIconElement::new(UiIcon::VCS)
                .xsmall()
                .text_color(p.primary),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.foreground)
                .child(badge.branch.unwrap_or_else(|| "-".into())),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(format!("↑{} ↓{}", badge.ahead, badge.behind)),
        )
        .into_any_element()
}

/// 日志工作区三栏主体视图。
pub(crate) fn render_git_log_view(
    workspace: Entity<GitWorkspace>,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let tab_id = workspace.read(cx).tab_id;
    let state = &workspace.read(cx).state;

    if state.not_repo {
        let error = state
            .log
            .error
            .clone()
            .or_else(|| state.status_error.clone())
            .unwrap_or_else(|| rust_i18n::t!("git.repo.not_repo").to_string());
        let init_running = state.init_running;
        return render_git_not_repo_state(error, init_running, workspace.clone(), p);
    }
    let filter_bar = log_view::render_filter_bar(state, workspace.clone(), p);
    let refs_open = state.refs_panel;
    let refs_width = state.refs_width;
    let details_width = state.details_width;
    let log_split = state.log_split.clone();
    let left = if refs_open {
        log_view::render_refs_panel(state, workspace.clone(), p, cx)
    } else {
        render_collapsed_refs_panel(workspace.clone())
    };
    let table = log_view::render_log_table(workspace.clone(), state, p);
    let right = log_view::render_right_panel(state, workspace, tab_id, p);
    let (left_width, left_min, left_max) = if refs_open {
        (refs_width, 180.0, 420.0)
    } else {
        (32.0, 32.0, 32.0)
    };
    // 中间栏：顶部是过滤条（文本/分支/用户/日期等），下方是提交图谱与历史表格。
    let center = v_flex()
        .size_full()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(filter_bar)
        .child(table);
    h_resizable(ElementId::Name(format!("git-log-split-{tab_id}").into()))
        .with_state(&log_split)
        .child(
            resizable_panel()
                .size(px(left_width))
                .size_range(px(left_min)..px(left_max))
                .fixed()
                .child(left),
        )
        .child(resizable_panel().child(center))
        .child(
            resizable_panel()
                .size(px(details_width))
                .size_range(px(320.0)..px(760.0))
                .fixed()
                .child(right),
        )
        .into_any_element()
}

/// 引用栏收起后的窄入口，保证三栏分栏组的首面板索引稳定。
fn render_collapsed_refs_panel(workspace: Entity<GitWorkspace>) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .pt_1()
        .child(
            Button::new("git-refs-toggle-open")
                .ghost()
                .xsmall()
                .child(AppIconElement::new(UiIcon::CHEVRON_RIGHT))
                .accessibility_label(rust_i18n::t!("git.refs.panel_toggle").to_string())
                .tooltip(rust_i18n::t!("git.refs.panel_toggle").to_string())
                .on_click(move |_, _, cx| {
                    workspace.update(cx, |ws, cx| ws.toggle_refs_panel(cx));
                }),
        )
        .into_any_element()
}

pub(crate) fn hint(text: String, p: &Palette) -> AnyElement {
    hint_colored(text, p.muted_foreground)
}

pub(crate) fn hint_colored(text: String, color: Hsla) -> AnyElement {
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p_3()
        .text_sm()
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// 「不是 Git 仓库」空态：原因 + 补初始化入口。
///
/// 概览与日志两个视图共用入口：未初始化的项目在任一屏都应能直接补初始化，
/// 不需要用户先切到另一屏再找按钮。
pub(crate) fn render_git_not_repo_state(
    error: String,
    init_running: bool,
    workspace: Entity<GitWorkspace>,
    p: &Palette,
) -> AnyElement {
    let label = if init_running {
        rust_i18n::t!("git.repo.init_running").to_string()
    } else {
        rust_i18n::t!("git.repo.init_action").to_string()
    };
    v_flex()
        .flex_1()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            AppIconElement::new(UiIcon::VCS)
                .with_size(px(30.0))
                .text_color(p.muted_foreground),
        )
        .child(
            div()
                .max_w(px(520.0))
                .text_sm()
                .text_color(p.muted_foreground)
                .child(error),
        )
        .child(
            Button::new("git-init-project")
                .primary()
                .small()
                .label(label)
                .disabled(init_running)
                .on_click(move |_, _, cx| {
                    workspace.update(cx, |ws, cx| ws.init_project(cx));
                }),
        )
        .into_any_element()
}
