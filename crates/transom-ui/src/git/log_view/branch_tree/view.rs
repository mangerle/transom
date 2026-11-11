//! 引用栏分支树的 GPUI 渲染层。
//!
//! 设计原理（Why）：这里只负责把已构建好的树节点映射为行、图标、菜单和点击行为；
//! 分类与排序规则留在 `model.rs`，避免渲染代码再次解释引用名称。

use super::super::current_head_name;
use super::super::menus::{
    LocalBranchMenuTarget, RemoteBranchMenuTarget, TagMenuTarget, render_local_branch_menu,
    render_remote_branch_menu, render_tag_menu,
};
use super::model::{RefGroup, RefNode, RefNodeKind, build_nodes};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{LogScope, TabId};
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;
use transom_ui_icons::render_app_icon;

type State = crate::git::GitWorkspaceState;

/// 渲染一个引用分组及其树内容。
pub(crate) fn render_ref_group(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    group: RefGroup,
    query: &str,
    p: &Palette,
) -> AnyElement {
    let searching = !query.is_empty();
    let expanded = searching
        || state
            .refs_expanded
            .get(group.key())
            .copied()
            .unwrap_or_else(|| group.default_expanded());
    let nodes = build_nodes(state, group, query);
    let header_app = app.clone();
    let header_key = group.key();
    let default_expanded = group.default_expanded();
    let title = group.title();

    let mut block = v_flex().child(
        h_flex()
            .id(ElementId::Name(format!("git-refs-{}", group.key()).into()))
            .px_1()
            .py_0p5()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(
                AppIconElement::new(if expanded {
                    UiIcon::CHEVRON_DOWN
                } else {
                    UiIcon::CHEVRON_RIGHT
                })
                .xsmall(),
            )
            .child(div().child(title))
            .on_click(move |_, _, cx| {
                header_app.update(cx, |this, cx| {
                    this.toggle_git_refs_group(header_key, default_expanded, cx);
                });
            }),
    );
    if expanded {
        for node in &nodes {
            block = block.child(render_node(state, &app, tab_id, node, 1, searching, p));
        }
    }
    block.into_any_element()
}

/// 递归渲染树节点。
fn render_node(
    state: &State,
    app: &Entity<AppShell>,
    tab_id: TabId,
    node: &RefNode,
    depth: usize,
    searching: bool,
    p: &Palette,
) -> AnyElement {
    if node.kind == RefNodeKind::Folder {
        return render_folder_node(state, app, tab_id, node, depth, searching, p);
    }
    render_branch_node(state, app, tab_id, node, depth, p)
}

/// 渲染可折叠的文件夹节点。
fn render_folder_node(
    state: &State,
    app: &Entity<AppShell>,
    tab_id: TabId,
    node: &RefNode,
    depth: usize,
    searching: bool,
    p: &Palette,
) -> AnyElement {
    let expanded = searching
        || state
            .refs_expanded
            .get(&node.key)
            .copied()
            .unwrap_or(node.default_expanded);
    let toggle_app = app.clone();
    let toggle_key = node.key.clone();
    let default_expanded = node.default_expanded;
    let mut block = v_flex().child(
        h_flex()
            .id(ElementId::Name(node.key.clone().into()))
            .pl(px(row_indent(depth)))
            .pr_1()
            .py_0p5()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .rounded(px(3.0))
            .hover(|style| style.bg(p.list_hover))
            .child(
                AppIconElement::new(if expanded {
                    UiIcon::CHEVRON_DOWN
                } else {
                    UiIcon::CHEVRON_RIGHT
                })
                .xsmall(),
            )
            .child(
                AppIconElement::new(UiIcon::FOLDER)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(node.label.clone()),
            )
            .on_click(move |_, _, cx| {
                toggle_app.update(cx, |this, cx| {
                    this.toggle_git_refs_group(&toggle_key, default_expanded, cx);
                });
            }),
    );
    if expanded {
        for child in &node.children {
            block = block.child(render_node(
                state,
                app,
                tab_id,
                child,
                depth + 1,
                searching,
                p,
            ));
        }
    }
    block.into_any_element()
}

/// 渲染分支或标签叶子节点。
fn render_branch_node(
    state: &State,
    app: &Entity<AppShell>,
    tab_id: TabId,
    node: &RefNode,
    depth: usize,
    p: &Palette,
) -> AnyElement {
    let Some(name) = node.full_name.as_ref() else {
        return div().into_any_element();
    };
    let is_selected = match &state.query.scope {
        LogScope::CurrentBranch => node.kind == RefNodeKind::LocalBranch && node.current,
        LogScope::AllRefs => false,
        LogScope::Ref(scope_name) => scope_name == name,
    };
    let current_branch = current_head_name(state);
    let upstream = state
        .branches
        .iter()
        .find(|branch| branch.name == *name)
        .and_then(|branch| branch.upstream.clone());
    let ahead = (node.kind == RefNodeKind::LocalBranch && node.current)
        .then(|| state.status.as_ref().map(|status| status.ahead))
        .flatten()
        .filter(|ahead| *ahead > 0);
    let target_scope = if node.kind == RefNodeKind::Head {
        LogScope::CurrentBranch
    } else {
        LogScope::Ref(name.clone())
    };
    let click_app = app.clone();
    let menu_app = app.clone();
    let menu_name = name.clone();
    let menu_project = state.project.clone();
    let menu_current = current_branch.clone();
    let menu_is_current = node.kind == RefNodeKind::Head || node.current;
    let menu_kind = node.kind;

    h_flex()
        .id(ElementId::Name(node.key.clone().into()))
        .pl(px(row_indent(depth)))
        .pr_1()
        .py_0p5()
        .gap_1()
        .items_center()
        .cursor_pointer()
        .rounded(px(3.0))
        .when(is_selected, |style| style.bg(p.list_active))
        .when(!is_selected, |style| {
            style.hover(|hover| hover.bg(p.list_hover))
        })
        .child(branch_icon(node, is_selected, p))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .truncate()
                .text_sm()
                .font_weight(if node.current || is_selected {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if node.current || is_selected {
                    p.foreground
                } else {
                    p.muted_foreground
                })
                .child(node.label.clone()),
        )
        .when_some(ahead, |el, ahead| {
            el.child(
                h_flex()
                    .gap_0p5()
                    .items_center()
                    .text_xs()
                    .text_color(p.primary)
                    .child(AppIconElement::new(UiIcon::UP).xsmall())
                    .child(ahead.to_string()),
            )
        })
        .on_click(move |_, _, cx| {
            let scope = target_scope.clone();
            click_app.update(cx, |this, cx| {
                this.git_set_log_scope(tab_id, scope, cx);
            });
        })
        .context_menu(move |menu, _window, _cx| match menu_kind {
            RefNodeKind::Head | RefNodeKind::LocalBranch => render_local_branch_menu(
                menu,
                menu_app.clone(),
                LocalBranchMenuTarget {
                    project: menu_project.clone(),
                    tab_id,
                    branch: menu_name.clone(),
                    current_branch: menu_current.clone(),
                    upstream: upstream.clone(),
                    is_current: menu_is_current,
                },
            ),
            RefNodeKind::RemoteBranch => render_remote_branch_menu(
                menu,
                menu_app.clone(),
                RemoteBranchMenuTarget {
                    project: menu_project.clone(),
                    tab_id,
                    remote_branch: menu_name.clone(),
                    current_branch: menu_current.clone(),
                },
            ),
            RefNodeKind::Tag => render_tag_menu(
                menu,
                menu_app.clone(),
                TagMenuTarget {
                    project: menu_project.clone(),
                    tab_id,
                    tag: menu_name.clone(),
                    current_branch: menu_current.clone(),
                },
            ),
            RefNodeKind::Folder => menu,
        })
        .into_any_element()
}

/// 叶子节点图标与颜色。
fn branch_icon(node: &RefNode, is_selected: bool, p: &Palette) -> AnyElement {
    match node.kind {
        RefNodeKind::Head => AppIconElement::new(UiIcon::VCS)
            .xsmall()
            .text_color(if is_selected { p.primary } else { p.success })
            .into_any_element(),
        RefNodeKind::LocalBranch => AppIconElement::new(UiIcon::VCS)
            .xsmall()
            .text_color(if is_selected {
                p.primary
            } else if node.current {
                p.warning
            } else {
                p.primary
            })
            .into_any_element(),
        RefNodeKind::RemoteBranch => AppIconElement::new(UiIcon::VCS)
            .xsmall()
            .text_color(if node.favorite {
                p.warning
            } else if is_selected {
                p.primary
            } else {
                p.muted_foreground
            })
            .into_any_element(),
        RefNodeKind::Tag => render_app_icon(UiIcon::TAG.into(), 14.0),
        RefNodeKind::Folder => render_app_icon(UiIcon::FOLDER.into(), 14.0),
    }
}

/// 树层级缩进。
fn row_indent(depth: usize) -> f32 {
    4.0 + depth as f32 * 12.0
}
