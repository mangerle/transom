//! 引用栏：HEAD / 本地 / 远程 / 标签 / 工作树五组，以及它们的折叠与当前头指针标记。
//!
//! 设计原理（Why）：引用栏按 IDEA 分组，默认展开「HEAD」「本地」与「远程」；
//! 标签与工作树保持紧凑。工作树与引用同栏是因为在 IDEA 里它们就是同一棵树里的条目。

use super::branch_tree::{RefGroup, render_ref_group};
use super::ref_toolbar::render_refs_action_bar;
use super::worktrees_group::render_worktree_group;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{RefKind, TabId};

type State = crate::git::GitWorkspaceState;

use gpui_kit::component::input::Input;

/// 左栏引用树（对齐 IDEA 结构：左侧动作条 + 顶部搜索过滤 + 带角标的树）。
pub(crate) fn render_refs_panel(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let query = state
        .refs_search_input
        .read(cx)
        .value()
        .trim()
        .to_lowercase();
    let search_input = state.refs_search_input.clone();
    // 1. 左侧垂直微型动作条（对齐 IDEA 左侧窄条）
    let action_bar = render_refs_action_bar(state, app.clone(), tab_id, p);

    // 2. 右侧树形内容与搜索输入框
    let tree_content = v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .p_1()
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(
                    Input::new(&search_input).xsmall().prefix(
                        Icon::new(IconName::Search)
                            .xsmall()
                            .text_color(p.muted_foreground),
                    ),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_1()
                .gap_0p5()
                .child(render_ref_group(
                    state,
                    app.clone(),
                    tab_id,
                    RefGroup::Head,
                    &query,
                    p,
                ))
                .child(render_ref_group(
                    state,
                    app.clone(),
                    tab_id,
                    RefGroup::Local,
                    &query,
                    p,
                ))
                .child(render_ref_group(
                    state,
                    app.clone(),
                    tab_id,
                    RefGroup::Remote,
                    &query,
                    p,
                ))
                .child(render_ref_group(
                    state,
                    app.clone(),
                    tab_id,
                    RefGroup::Tags,
                    &query,
                    p,
                ))
                .child(render_worktree_group(&app, state, p)),
        );

    h_flex()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(action_bar)
        .child(tree_content)
        .into_any_element()
}

/// 引用装饰徽标（IDEA 风格半透明圆角细线胶囊）。
pub(crate) fn ref_chip(kind: RefKind, name: &str, p: &Palette) -> AnyElement {
    let color = match kind {
        RefKind::HeadAt => p.primary,
        RefKind::Branch => p.success,
        RefKind::RemoteBranch => p.muted_foreground,
        RefKind::Tag => p.warning,
    };
    h_flex()
        .h(px(16.0))
        .px_1()
        .items_center()
        .rounded(px(3.0))
        .bg(color.opacity(0.12))
        .border_1()
        .border_color(color.opacity(0.35))
        .text_xs()
        .text_color(color)
        .child(name.to_string())
        .into_any_element()
}
