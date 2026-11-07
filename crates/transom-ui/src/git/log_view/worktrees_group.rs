//! 工作树引用分组渲染（工作树列表与操作菜单）。
//!
//! 设计原理（Why）：
//! - 将工作树分组视图从引用栏中解耦，使 `log_refs.rs` 和本文件生产代码均严格保持在 500 行以内；
//! - 明确支持工作树路径展示、锁定状态标识、快速切换以及右键加锁/解锁/删除。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::widgets::{MenuItem, menu_item};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

type State = crate::git::GitWorkspaceState;

/// 工作树分组：一行一个工作树，右键可锁定或删除。
pub(crate) fn render_worktree_group(
    app: &Entity<AppShell>,
    state: &State,
    p: &Palette,
) -> AnyElement {
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
                .context_menu({
                    let menu_project = state.project.clone();
                    move |menu, _window, _cx| {
                        let lock_app = menu_app.clone();
                        let lock_project = menu_project.clone();
                        let lock_owned = lock_path.clone();
                        let remove_app = menu_app.clone();
                        let remove_project = menu_project.clone();
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
                                let project = lock_project.clone();
                                lock_app.update(cx, |this, cx| {
                                    this.git_set_project_worktree_locked(
                                        &project, path, !locked, cx,
                                    );
                                });
                            },
                        ))
                        .item(PopupMenuItem::separator())
                        .item(
                            MenuItem::new(
                                rust_i18n::t!("git.menu.remove_worktree").to_string(),
                                IconName::Trash,
                            )
                            .disabled(is_main || locked)
                            .render(move |_, _, cx| {
                                let path = remove_owned.clone();
                                let project = remove_project.clone();
                                remove_app.update(cx, |this, cx| {
                                    this.git_remove_worktree(&project, path, false, cx);
                                });
                            }),
                        )
                    }
                }),
        );
    }
    block.into_any_element()
}
