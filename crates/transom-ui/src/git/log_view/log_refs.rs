//! 引用栏：分支 / 标签 / 工作树三组，以及它们的折叠与当前头指针标记。
//!
//! 设计原理（Why）：引用栏按 IDEA 分组，默认只展开「本地分支」——分支多的仓库
//! 全展开会把可视区吃光；工作树与引用同栏是因为在 IDEA 里它们就是同一棵树里的条目。

use super::menus::{
    LocalBranchMenuTarget, RemoteBranchMenuTarget, TagMenuTarget, render_local_branch_menu,
    render_remote_branch_menu, render_tag_menu,
};
use super::worktrees_group::render_worktree_group;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{HeadState, RefKind, TabId};

type State = crate::git::GitWorkspaceState;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;

/// 左栏引用树（对齐 IDEA 结构：左侧动作条 + 顶部搜索过滤 + 带角标的树）。
pub(crate) fn render_refs_panel(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    width: f32,
    p: &Palette,
) -> AnyElement {
    let local: Vec<String> = state
        .branches
        .iter()
        .filter(|branch| !branch.remote)
        .map(|branch| branch.name.clone())
        .collect();
    let remote: Vec<String> = state
        .branches
        .iter()
        .filter(|branch| branch.remote)
        .map(|branch| branch.name.clone())
        .collect();
    let tags: Vec<String> = state.tags.iter().map(|tag| tag.name.clone()).collect();
    let head = current_head_name(state);

    let search_input = state.refs_search_input.clone();
    let toggle_app = app.clone();
    let refresh_project = state.project.clone();
    let pull_project = state.project.clone();

    // 1. 左侧垂直微型动作条（对齐 IDEA 左侧窄条）
    let action_bar = v_flex()
        .w(px(26.0))
        .flex_none()
        .h_full()
        .items_center()
        .py_1()
        .gap_1()
        .bg(p.muted)
        .border_color(p.divider(p.border))
        .border_r_1()
        .child(
            Button::new("git-refs-collapse-btn")
                .ghost()
                .xsmall()
                .icon(IconName::PanelLeftClose)
                .tooltip(rust_i18n::t!("common.close").to_string())
                .on_click(move |_, _, cx| {
                    toggle_app.update(cx, |this, cx| {
                        this.git_toggle_refs_panel(tab_id, cx);
                    });
                }),
        )
        .child(
            Button::new("git-refs-new-branch-btn")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .tooltip(rust_i18n::t!("git.refs.new_branch").to_string())
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.git_create_branch_from("HEAD", window, cx);
                        });
                    }
                }),
        )
        .child(
            Button::new("git-refs-refresh-btn")
                .ghost()
                .xsmall()
                .icon(IconName::RotateCw)
                .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.git_refresh(&refresh_project, cx);
                            this.git_load_log(tab_id, 0, cx);
                        });
                    }
                }),
        )
        .child(
            Button::new("git-refs-pull-btn")
                .ghost()
                .xsmall()
                .icon(IconName::ArrowDownToLine)
                .tooltip(rust_i18n::t!("git.toolbar.pull").to_string())
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.git_remote_action(
                                &pull_project,
                                crate::git::actions::GitRemoteAction::Pull { rebase: false },
                                cx,
                            );
                        });
                    }
                }),
        );

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
                .child(Input::new(&search_input).xsmall()),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_1()
                .gap_0p5()
                .child(render_refs_group(
                    &app,
                    state,
                    tab_id,
                    RefsGroupConfig {
                        title: "HEAD",
                        names: &head,
                        default_expanded: true,
                        is_local_branches: false,
                        kind: RefKind::HeadAt,
                    },
                    p,
                ))
                .child(render_refs_group(
                    &app,
                    state,
                    tab_id,
                    RefsGroupConfig {
                        title: rust_i18n::t!("git.refs.local").as_ref(),
                        names: &local,
                        default_expanded: true,
                        is_local_branches: true,
                        kind: RefKind::Branch,
                    },
                    p,
                ))
                .child(render_refs_group(
                    &app,
                    state,
                    tab_id,
                    RefsGroupConfig {
                        title: rust_i18n::t!("git.refs.remote").as_ref(),
                        names: &remote,
                        default_expanded: false,
                        is_local_branches: false,
                        kind: RefKind::RemoteBranch,
                    },
                    p,
                ))
                .child(render_refs_group(
                    &app,
                    state,
                    tab_id,
                    RefsGroupConfig {
                        title: rust_i18n::t!("git.refs.tags").as_ref(),
                        names: &tags,
                        default_expanded: false,
                        is_local_branches: false,
                        kind: RefKind::Tag,
                    },
                    p,
                ))
                .child(render_worktree_group(&app, state, p)),
        );

    h_flex()
        .w(px(width))
        .flex_none()
        .min_h_0()
        .h_full()
        .border_color(p.divider(p.border))
        .border_r_1()
        .child(action_bar)
        .child(tree_content)
        .into_any_element()
}

/// 当前头指针在引用栏里的显示项。
fn current_head_name(state: &State) -> Vec<String> {
    let Some(status) = state.status.as_ref() else {
        return Vec::new();
    };
    match &status.head {
        HeadState::OnBranch(name) | HeadState::Initial { branch: name } => vec![name.clone()],
        HeadState::Detached => Vec::new(),
    }
}

/// 引用分组渲染配置。
struct RefsGroupConfig<'a> {
    title: &'a str,
    names: &'a [String],
    default_expanded: bool,
    is_local_branches: bool,
    kind: RefKind,
}

fn render_refs_group(
    app: &Entity<AppShell>,
    state: &State,
    tab_id: TabId,
    cfg: RefsGroupConfig<'_>,
    p: &Palette,
) -> AnyElement {
    let key = cfg.title.to_string();
    let expanded = state
        .refs_expanded
        .get(&key)
        .copied()
        .unwrap_or(cfg.default_expanded);
    let head_app = app.clone();
    let head_key = key.clone();
    let current = current_head_name(state);

    let mut block = v_flex().child(
        h_flex()
            .id(ElementId::Name(format!("git-refs-{key}").into()))
            .px_1()
            .py_0p5()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall(),
            )
            .child(div().child(cfg.title.to_string()))
            .on_click(move |_, _, cx| {
                head_app.update(cx, |this, cx| {
                    this.toggle_git_refs_group(&head_key, cx);
                });
            }),
    );
    let group_kind = cfg.kind;
    let current_branch_name = current.first().cloned();
    if expanded {
        for name in cfg.names {
            let display = name.clone();
            let click_name = name.clone();
            let menu_name = name.clone();
            let click_app = app.clone();
            let dbl_click_app = app.clone();
            let menu_app = app.clone();
            let cur_branch = current_branch_name.clone();
            let is_head = current.iter().any(|head| head == name);

            // 判断中间提交历史面板是否正在展示当前引用的提交
            let is_selected = match &state.query.scope {
                transom_core::LogScope::CurrentBranch => {
                    group_kind == RefKind::HeadAt || (cfg.is_local_branches && is_head)
                }
                transom_core::LogScope::AllRefs => false,
                transom_core::LogScope::Ref(scope_name) => scope_name == name,
            };

            // 本地分支领先/落后角标
            let ahead_badge = if cfg.is_local_branches && is_head {
                state
                    .status
                    .as_ref()
                    .and_then(|s| if s.ahead > 0 { Some(s.ahead) } else { None })
            } else {
                None
            };
            let is_star_branch =
                !cfg.is_local_branches && (name == "origin/master" || name == "origin/main");

            let target_scope = if group_kind == RefKind::HeadAt {
                transom_core::LogScope::CurrentBranch
            } else {
                transom_core::LogScope::Ref(click_name.clone())
            };
            let dbl_name = click_name.clone();
            let can_checkout = cfg.is_local_branches;

            let menu_project = state.project.clone();
            block = block.child(
                h_flex()
                    .id(ElementId::Name(format!("git-ref-{display}").into()))
                    .pl_3()
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
                    .when(is_star_branch, |el| {
                        el.child(Icon::new(IconName::Star).xsmall().text_color(p.warning))
                    })
                    .child(
                        Icon::new(if is_head && cfg.is_local_branches {
                            IconName::Check
                        } else {
                            IconName::GitBranch
                        })
                        .xsmall()
                        .text_color(if is_selected {
                            p.primary
                        } else if is_head {
                            p.success
                        } else {
                            p.muted_foreground
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_xs()
                            .font_weight(if is_head || is_selected {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_selected || is_head {
                                p.foreground
                            } else {
                                p.muted_foreground
                            })
                            .child(display),
                    )
                    .when_some(ahead_badge, |el, ahead| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(p.primary)
                                .font_weight(FontWeight::BOLD)
                                .child(format!("↗ {ahead}")),
                        )
                    })
                    .on_click(move |event, _, cx| {
                        if event.click_count() >= 2 && can_checkout {
                            // 双击本地分支执行检出（对齐 openidea）
                            dbl_click_app.update(cx, |this, cx| {
                                this.git_checkout_ref(&dbl_name, cx);
                            });
                        } else {
                            // 单击切换中间面板的提交日志查看范围（对齐 openidea）
                            let scope = target_scope.clone();
                            click_app.update(cx, |this, cx| {
                                this.git_set_log_scope(tab_id, scope, cx);
                            });
                        }
                    })
                    .context_menu(move |menu, _window, _cx| match group_kind {
                        RefKind::Branch | RefKind::HeadAt => render_local_branch_menu(
                            menu,
                            menu_app.clone(),
                            LocalBranchMenuTarget {
                                project: menu_project.clone(),
                                tab_id,
                                branch: menu_name.clone(),
                                current_branch: cur_branch.clone(),
                                upstream: None,
                                is_current: is_head,
                            },
                        ),
                        RefKind::RemoteBranch => render_remote_branch_menu(
                            menu,
                            menu_app.clone(),
                            RemoteBranchMenuTarget {
                                project: menu_project.clone(),
                                tab_id,
                                remote_branch: menu_name.clone(),
                                current_branch: cur_branch.clone(),
                            },
                        ),
                        RefKind::Tag => render_tag_menu(
                            menu,
                            menu_app.clone(),
                            TagMenuTarget {
                                project: menu_project.clone(),
                                tab_id,
                                tag: menu_name.clone(),
                                current_branch: cur_branch.clone(),
                            },
                        ),
                    }),
            );
        }
    }
    block.into_any_element()
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
