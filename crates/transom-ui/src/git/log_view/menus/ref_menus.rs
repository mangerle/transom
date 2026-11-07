//! 分支与标签引用的右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 将本地分支、远程分支和标签三者的右键菜单分立，彻底解决此前混为一谈导致的
//!   菜单项冗余或语义不精准的问题；
//! - 完整对齐 IDEA 的项顺序、占位分支名与快捷键提示（推送 Ctrl+Shift+K、重命名 F2、删除 Delete）。

use crate::app::AppShell;
use crate::widgets::{MenuItem, menu_item, menu_item_with_shortcut};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// 本地分支右键上下文菜单（对齐 IDEA 本地分支菜单）。
pub(crate) fn render_local_branch_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    branch: String,
    current_branch: Option<String>,
    upstream: Option<String>,
    is_current: bool,
) -> PopupMenu {
    let app_new_branch = app.clone();
    let app_worktree = app.clone();
    let app_pull = app.clone();
    let app_push = app.clone();
    let app_rename = app.clone();
    let app_delete = app.clone();
    let app_diff = app.clone();

    let name_new_branch = branch.clone();
    let name_worktree = branch.clone();
    let name_rename = branch.clone();
    let name_delete = branch.clone();
    let name_diff = branch.clone();

    let upstream_display = upstream.unwrap_or_else(|| {
        format!(
            "origin/{}",
            current_branch.as_deref().unwrap_or(branch.as_str())
        )
    });

    menu
        // 1. 从 '{branch}' 新建分支...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_branch_from_ref", branch = branch.as_str()).to_string(),
            IconName::GitBranchPlus,
            move |_, window, cx| {
                let start = name_new_branch.clone();
                app_new_branch.update(cx, |this, cx| {
                    this.git_create_branch_from(&start, window, cx);
                });
            },
        ))
        // 2. 显示与工作树的差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
            IconName::GitCompare,
            move |_, _, cx| {
                let target = name_diff.clone();
                app_diff.update(cx, |this, cx| {
                    this.git_checkout_ref(&target, cx);
                });
            },
        ))
        // 3. 从 '{branch}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_worktree_from_ref", branch = branch.as_str()).to_string(),
            IconName::FolderGit,
            move |_, window, cx| {
                let start = name_worktree.clone();
                app_worktree.update(cx, |this, cx| {
                    this.git_new_worktree_from(&start, window, cx);
                });
            },
        ))
        .item(PopupMenuItem::separator())
        // 4. 更新
        .item(menu_item(
            rust_i18n::t!("git.menu.update").to_string(),
            IconName::RotateCw,
            move |_, _, cx| {
                app_pull.update(cx, |this, cx| this.git_pull_current(cx));
            },
        ))
        // 5. 推送... (Ctrl+Shift+K)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.push").to_string(),
            IconName::ArrowUpFromLine,
            "Ctrl+Shift+K",
            move |_, _, cx| {
                app_push.update(cx, |this, cx| this.git_push_current(cx));
            },
        ))
        // 6. 跟踪的分支 '{upstream}'
        .item(
            MenuItem::new(
                rust_i18n::t!(
                    "git.menu.tracked_branch",
                    upstream = upstream_display.as_str()
                )
                .to_string(),
                IconName::GitBranch,
            )
            .disabled(false)
            .render(|_, _, _| {}),
        )
        .item(PopupMenuItem::separator())
        // 7. 重命名... (F2)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.rename_branch").to_string(),
            IconName::Pencil,
            "F2",
            move |_, window, cx| {
                let old = name_rename.clone();
                app_rename.update(cx, |this, cx| {
                    let Some(project) = this.git_project_of_tab(None) else {
                        return;
                    };
                    this.open_git_name_modal(
                        &project,
                        crate::git::GitNamePurpose::RenameBranch { old: old.clone() },
                        old,
                        window,
                        cx,
                    );
                });
            },
        ))
        // 8. 删除(D) (仅非当前检出分支允许删除)
        .when(!is_current, |m| {
            m.item(menu_item_with_shortcut(
                rust_i18n::t!("git.menu.delete_ref").to_string(),
                IconName::Trash,
                "Delete",
                move |_, _, cx| {
                    let name = name_delete.clone();
                    app_delete.update(cx, |this, cx| this.git_delete_ref(&name, cx));
                },
            ))
        })
}

/// 远程分支右键上下文菜单（对齐 IDEA 远程分支菜单）。
pub(crate) fn render_remote_branch_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    remote_branch: String,
    current_branch: Option<String>,
) -> PopupMenu {
    let cur = current_branch.unwrap_or_else(|| "main".to_string());
    let app_checkout = app.clone();
    let app_new_branch = app.clone();
    let app_rebase_into = app.clone();
    let app_merge_into = app.clone();
    let app_worktree = app.clone();
    let app_delete = app.clone();

    let name_checkout = remote_branch.clone();
    let name_new_branch = remote_branch.clone();
    let name_rebase = remote_branch.clone();
    let name_merge = remote_branch.clone();
    let name_worktree = remote_branch.clone();
    let name_delete = remote_branch.clone();

    menu
        // 1. 签出
        .item(menu_item(
            rust_i18n::t!("git.menu.checkout").to_string(),
            IconName::ArrowRight,
            move |_, _, cx| {
                let name = name_checkout.clone();
                app_checkout.update(cx, |this, cx| {
                    this.git_checkout_ref(&name, cx);
                });
            },
        ))
        // 2. 从 '{remote_branch}' 新建分支...
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.new_branch_from_ref",
                branch = remote_branch.as_str()
            )
            .to_string(),
            IconName::GitBranchPlus,
            move |_, window, cx| {
                let start = name_new_branch.clone();
                app_new_branch.update(cx, |this, cx| {
                    this.git_create_branch_from(&start, window, cx);
                });
            },
        ))
        // 3. 签出并变基到 '{current}'
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.checkout_and_rebase", current = cur.as_str()).to_string(),
                IconName::GitCompareArrows,
            )
            .render(|_, _, _| {}),
        )
        // 4. 与 '{current}' 比较
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.compare_with_branch", current = cur.as_str()).to_string(),
                IconName::GitCompare,
            )
            .render(|_, _, _| {}),
        )
        // 5. 显示与工作树的差异
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
                IconName::GitCompare,
            )
            .render(|_, _, _| {}),
        )
        .item(PopupMenuItem::separator())
        // 6. 将 '{current}' 变基到 '{remote_branch}'
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.rebase_current_onto_ref",
                current = cur.as_str(),
                branch = remote_branch.as_str()
            )
            .to_string(),
            IconName::GitCompareArrows,
            move |_, _, cx| {
                let upstream = name_rebase.clone();
                app_rebase_into.update(cx, |this, cx| {
                    this.git_history_on_current(
                        crate::git::actions::GitHistoryAction::Rebase(upstream),
                        cx,
                    );
                });
            },
        ))
        // 7. 将 '{remote_branch}' 合并到 '{current}' 中
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.merge_ref_into_current",
                branch = remote_branch.as_str(),
                current = cur.as_str()
            )
            .to_string(),
            IconName::GitMerge,
            move |_, _, cx| {
                let reference = name_merge.clone();
                app_merge_into.update(cx, |this, cx| {
                    this.git_history_on_current(
                        crate::git::actions::GitHistoryAction::Merge(reference),
                        cx,
                    );
                });
            },
        ))
        // 8. 从 '{remote_branch}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.new_worktree_from_ref",
                branch = remote_branch.as_str()
            )
            .to_string(),
            IconName::FolderGit,
            move |_, window, cx| {
                let branch = name_worktree.clone();
                app_worktree.update(cx, |this, cx| {
                    this.git_new_worktree_from(&branch, window, cx);
                });
            },
        ))
        // 9. 使用“变基”拉入 '{current}'
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.pull_with_rebase", current = cur.as_str()).to_string(),
                IconName::RotateCw,
            )
            .render(|_, _, _| {}),
        )
        // 10. 使用“合并”拉入 '{current}'
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.pull_with_merge", current = cur.as_str()).to_string(),
                IconName::RotateCw,
            )
            .render(|_, _, _| {}),
        )
        .item(PopupMenuItem::separator())
        // 11. 删除(D)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.delete_ref").to_string(),
            IconName::Trash,
            "Delete",
            move |_, _, cx| {
                let name = name_delete.clone();
                app_delete.update(cx, |this, cx| this.git_delete_ref(&name, cx));
            },
        ))
}

/// 标签右键上下文菜单（对齐 IDEA 标签菜单）。
pub(crate) fn render_tag_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    tag: String,
    current_branch: Option<String>,
) -> PopupMenu {
    let cur = current_branch.unwrap_or_else(|| "main".to_string());
    let app_checkout = app.clone();
    let app_merge = app.clone();
    let app_worktree = app.clone();
    let app_push = app.clone();
    let app_delete = app.clone();

    let name_checkout = tag.clone();
    let name_merge = tag.clone();
    let name_worktree = tag.clone();
    let name_delete = tag.clone();

    menu
        // 1. 签出
        .item(menu_item(
            rust_i18n::t!("git.menu.checkout").to_string(),
            IconName::ArrowRight,
            move |_, _, cx| {
                let name = name_checkout.clone();
                app_checkout.update(cx, |this, cx| {
                    this.git_checkout_ref(&name, cx);
                });
            },
        ))
        // 2. 显示与工作树的差异
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
                IconName::GitCompare,
            )
            .render(|_, _, _| {}),
        )
        // 3. 将 '{tag}' 合并到 '{current}' 中
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.merge_tag_into",
                tag = tag.as_str(),
                branch = cur.as_str()
            )
            .to_string(),
            IconName::GitMerge,
            move |_, _, cx| {
                let reference = name_merge.clone();
                app_merge.update(cx, |this, cx| {
                    this.git_history_on_current(
                        crate::git::actions::GitHistoryAction::Merge(reference),
                        cx,
                    );
                });
            },
        ))
        // 4. 从 '{tag}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_worktree_from_ref", branch = tag.as_str()).to_string(),
            IconName::FolderGit,
            move |_, window, cx| {
                let branch = name_worktree.clone();
                app_worktree.update(cx, |this, cx| {
                    this.git_new_worktree_from(&branch, window, cx);
                });
            },
        ))
        // 5. 推送到 origin
        .item(menu_item(
            rust_i18n::t!("git.menu.push_to_remote", remote = "origin").to_string(),
            IconName::Upload,
            move |_, _, cx| {
                app_push.update(cx, |this, cx| this.git_push_current(cx));
            },
        ))
        // 6. 删除(D)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.delete_ref").to_string(),
            IconName::Trash,
            "Delete",
            move |_, _, cx| {
                let name = name_delete.clone();
                app_delete.update(cx, |this, cx| this.git_delete_ref(&name, cx));
            },
        ))
}
