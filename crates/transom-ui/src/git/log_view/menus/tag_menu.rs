//! 标签右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 标签项的右键菜单分组和快捷键口径；
//! - 贯穿确定性 `GitProject` 与 `TabId`，彻底根除跨标签页盲查项目失败导致的点击静默失效；
//! - 参数收敛为专用目标结构体，符合复杂度控制与参数对象规范。

use crate::app::AppShell;
use crate::widgets::{menu_item, menu_item_with_shortcut};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use transom_core::{GitProject, LogScope, TabId};

/// 标签菜单目标数据。
pub(crate) struct TagMenuTarget {
    pub(crate) project: GitProject,
    pub(crate) tab_id: TabId,
    pub(crate) tag: String,
    pub(crate) current_branch: Option<String>,
}

/// 标签右键上下文菜单（对齐 IDEA 标签菜单）。
pub(crate) fn render_tag_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    target: TagMenuTarget,
) -> PopupMenu {
    let TagMenuTarget {
        project,
        tab_id,
        tag,
        current_branch,
    } = target;

    let cur = current_branch.unwrap_or_else(|| "main".to_string());
    let app_checkout = app.clone();
    let app_new_branch = app.clone();
    let app_merge = app.clone();
    let app_worktree = app.clone();
    let app_push = app.clone();
    let app_delete = app.clone();
    let app_diff = app;

    let project_checkout = project.clone();
    let project_new_branch = project.clone();
    let project_merge = project.clone();
    let project_worktree = project.clone();
    let project_push = project.clone();
    let project_delete = project;

    let name_checkout = tag.clone();
    let name_new_branch = tag.clone();
    let name_merge = tag.clone();
    let name_worktree = tag.clone();
    let name_push = tag.clone();
    let name_delete = tag.clone();
    let name_diff = tag.clone();

    menu
        // 1. 签出
        .item(menu_item(
            rust_i18n::t!("git.menu.checkout").to_string(),
            IconName::ArrowRight,
            move |_, _, cx| {
                let name = name_checkout.clone();
                let project = project_checkout.clone();
                app_checkout.update(cx, |this, cx| {
                    this.git_checkout_project_ref(&project, &name, cx);
                });
            },
        ))
        // 2. 从 '{tag}' 新建分支...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_branch_from_ref", branch = tag.as_str()).to_string(),
            IconName::GitBranchPlus,
            move |_, window, cx| {
                let start = name_new_branch.clone();
                let project = project_new_branch.clone();
                app_new_branch.update(cx, |this, cx| {
                    this.git_create_project_branch_from(&project, &start, window, cx);
                });
            },
        ))
        // 3. 显示与工作树的差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
            IconName::GitCompare,
            move |_, _, cx| {
                let target = name_diff.clone();
                app_diff.update(cx, |this, cx| {
                    this.git_set_log_scope(tab_id, LogScope::Ref(target), cx);
                });
            },
        ))
        // 4. 将 '{tag}' 合并到 '{current}' 中
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
                let project = project_merge.clone();
                app_merge.update(cx, |this, cx| {
                    this.git_history_on_project(
                        &project,
                        crate::git::actions::GitHistoryAction::Merge(reference),
                        cx,
                    );
                });
            },
        ))
        // 5. 从 '{tag}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_worktree_from_ref", branch = tag.as_str()).to_string(),
            IconName::FolderGit,
            move |_, window, cx| {
                let branch = name_worktree.clone();
                let project = project_worktree.clone();
                app_worktree.update(cx, |this, cx| {
                    this.open_git_worktree_modal(project, &branch, window, cx);
                });
            },
        ))
        // 6. 推送到 origin
        .item(menu_item(
            rust_i18n::t!("git.menu.push_to_remote", remote = "origin").to_string(),
            IconName::Upload,
            move |_, _, cx| {
                let name = name_push.clone();
                let project = project_push.clone();
                app_push.update(cx, |this, cx| {
                    this.git_push_project(&project, Some(format!("refs/tags/{}", name)), cx);
                });
            },
        ))
        // 7. 删除(D)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.delete_ref").to_string(),
            IconName::Trash,
            "Delete",
            move |_, _, cx| {
                let name = name_delete.clone();
                let project = project_delete.clone();
                app_delete.update(cx, |this, cx| {
                    this.git_delete_project_ref(&project, &name, cx);
                });
            },
        ))
}
