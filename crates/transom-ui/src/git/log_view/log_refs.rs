//! 引用栏：分支 / 标签 / 工作树三组，以及它们的折叠与当前头指针标记。
//!
//! 设计原理（Why）：引用栏按 IDEA 分组，默认只展开「本地分支」——分支多的仓库
//! 全展开会把可视区吃光；工作树与引用同栏是因为在 IDEA 里它们就是同一棵树里的条目。

use super::log_menus::render_ref_menu;
use crate::app::AppShell;
use crate::palette::Palette;
use crate::widgets::{MenuItem, menu_item};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{HeadState, RefKind};

type State = crate::git::GitWorkspaceState;

/// 左栏引用树。
pub(crate) fn render_refs_panel(
    state: &State,
    app: Entity<AppShell>,
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
    v_flex()
        .w(px(width))
        .flex_none()
        .min_h_0()
        .overflow_y_scrollbar()
        .p_1()
        .gap_0p5()
        .border_color(p.divider(p.border))
        .border_r_1()
        .child(render_refs_group(
            &app, state, "HEAD", &head, true, false, p,
        ))
        .child(render_refs_group(
            &app,
            state,
            rust_i18n::t!("git.refs.local").as_ref(),
            &local,
            true,
            true,
            p,
        ))
        .child(render_refs_group(
            &app,
            state,
            rust_i18n::t!("git.refs.remote").as_ref(),
            &remote,
            false,
            false,
            p,
        ))
        .child(render_refs_group(
            &app,
            state,
            rust_i18n::t!("git.refs.tags").as_ref(),
            &tags,
            false,
            false,
            p,
        ))
        .child(render_worktree_group(&app, state, p))
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

fn render_refs_group(
    app: &Entity<AppShell>,
    state: &State,
    title: &str,
    names: &[String],
    default_expanded: bool,
    // renameable：只有本地分支可重命名，远程分支与标签不行。
    renameable: bool,
    p: &Palette,
) -> AnyElement {
    let key = title.to_string();
    let expanded = state
        .refs_expanded
        .get(&key)
        .copied()
        .unwrap_or(default_expanded);
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
            .child(div().child(title.to_string()))
            .on_click(move |_, _, cx| {
                head_app.update(cx, |this, cx| {
                    this.toggle_git_refs_group(&head_key, cx);
                });
            }),
    );
    if expanded {
        for name in names {
            let display = name.clone();
            let click_name = name.clone();
            let menu_name = name.clone();
            let click_app = app.clone();
            let menu_app = app.clone();
            block = block.child(
                h_flex()
                    .id(ElementId::Name(format!("git-ref-{display}").into()))
                    .pl_4()
                    .pr_1()
                    .py_0p5()
                    .gap_1()
                    .items_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(p.list_hover))
                    .child(
                        Icon::new(IconName::GitBranch)
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(if current.iter().any(|head| head == name) {
                                p.foreground
                            } else {
                                p.muted_foreground
                            })
                            .child(display),
                    )
                    .on_click(move |_, _, cx| {
                        click_app.update(cx, |this, cx| {
                            this.git_checkout_ref(&click_name, cx);
                        });
                    })
                    .context_menu(move |menu, _window, _cx| {
                        render_ref_menu(menu, menu_app.clone(), menu_name.clone(), renameable)
                    }),
            );
        }
    }
    block.into_any_element()
}

/// 工作树分组：一行一个工作树，右键可删除登记。
fn render_worktree_group(app: &Entity<AppShell>, state: &State, p: &Palette) -> AnyElement {
    let mut block = v_flex().child(
        div()
            .px_1()
            .py_0p5()
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("git.refs.worktrees").to_string()),
    );
    for worktree in &state.worktrees {
        let display = worktree
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| worktree.path.display().to_string());
        let subtitle = match &worktree.branch {
            Some(branch) => branch.clone(),
            None => worktree.head.chars().take(7).collect(),
        };
        // 锁定原因与可清理原因都按 git 给的原文显示：删不掉时用户要看的
        // 就是这句话，藏起来只会让人回来问「为什么点了没反应」。
        let notice = worktree
            .lock_reason
            .as_deref()
            .map(|reason| format!("{} · {reason}", rust_i18n::t!("git.worktree.locked")))
            .or_else(|| {
                worktree
                    .prune_reason
                    .as_ref()
                    .map(|reason| format!("{} · {reason}", rust_i18n::t!("git.worktree.prunable")))
            });
        let is_main = worktree.path == state.project.path || worktree.bare;
        let locked = worktree.locked;
        let path_text = worktree.path.display().to_string();
        let item_app = app.clone();
        let menu_app = app.clone();
        let remove_path = path_text.clone();
        let lock_path = path_text.clone();
        block = block.child(
            h_flex()
                .id(ElementId::Name(format!("git-worktree-{path_text}").into()))
                .pl_4()
                .pr_1()
                .py_0p5()
                .gap_1()
                .items_center()
                .cursor_pointer()
                .hover(|style| style.bg(p.list_hover))
                .child(
                    Icon::new(if locked {
                        IconName::Lock
                    } else {
                        IconName::FolderGit
                    })
                    .xsmall()
                    .text_color(if locked { p.warning } else { p.primary }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .text_xs()
                                .overflow_hidden()
                                .truncate()
                                .child(format!("{display} · {subtitle}")),
                        )
                        .when(notice.is_some(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .overflow_hidden()
                                    .text_color(p.muted_foreground)
                                    .child(notice.clone().unwrap_or_default()),
                            )
                        }),
                )
                .on_click({
                    let open_app = item_app.clone();
                    let open_path = path_text.clone();
                    move |_, _, cx| {
                        let path = open_path.clone();
                        open_app.update(cx, |this, cx| {
                            this.git_open_worktree_path(&path, cx);
                        });
                    }
                })
                .context_menu(move |menu, _window, _cx| {
                    let lock_app = menu_app.clone();
                    let lock_owned = lock_path.clone();
                    let remove_app = menu_app.clone();
                    let remove_owned = remove_path.clone();
                    let toggle_key = if locked {
                        "git.menu.unlock_worktree"
                    } else {
                        "git.menu.lock_worktree"
                    };
                    menu.item(menu_item(
                        rust_i18n::t!(toggle_key).to_string(),
                        IconName::Lock,
                        move |_, _, cx| {
                            let path = lock_owned.clone();
                            lock_app.update(cx, |this, cx| {
                                this.git_set_worktree_locked(path, !locked, cx);
                            });
                        },
                    ))
                    .item(PopupMenuItem::separator())
                    .item(
                        // 主工作树与裸仓库不是「可删除的工作树」，被锁住的工作树
                        // 也要先解锁：两种情况都置灰说明原因，而不是点下去吃一个 git 错误。
                        MenuItem::new(
                            rust_i18n::t!("git.menu.remove_worktree").to_string(),
                            IconName::Trash,
                        )
                        .disabled(is_main || locked)
                        .render(move |_, _, cx| {
                            let path = remove_owned.clone();
                            remove_app.update(cx, |this, cx| {
                                this.git_remove_current_worktree(path, false, cx);
                            });
                        }),
                    )
                }),
        );
    }
    block.into_any_element()
}

/// 引用装饰徽标。
pub(crate) fn ref_chip(kind: RefKind, name: &str, p: &Palette) -> AnyElement {
    let color = match kind {
        RefKind::HeadAt => p.primary,
        RefKind::Branch => p.success,
        RefKind::RemoteBranch => p.secondary_foreground,
        RefKind::Tag => p.warning,
    };
    div()
        .px_1()
        .rounded(px(2.0))
        .border_1()
        .border_color(color)
        .text_xs()
        .text_color(color)
        .child(name.to_string())
        .into_any_element()
}
