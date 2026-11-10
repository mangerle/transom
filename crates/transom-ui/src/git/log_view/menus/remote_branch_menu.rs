//! 远程分支右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 远程分支项的右键菜单分组口径；
//! - 贯穿确定性 `GitProject` 与 `TabId`，彻底根除跨标签页盲查项目失败导致的点击静默失效；
//! - 参数收敛为专用目标结构体，符合复杂度控制与参数对象规范。

use crate::app::AppShell;
use crate::widgets::menu_item;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{GitProject, LogScope, TabId};

/// 远程分支菜单目标数据。
pub(crate) struct RemoteBranchMenuTarget {
    pub(crate) project: GitProject,
    pub(crate) tab_id: TabId,
    pub(crate) remote_branch: String,
    pub(crate) current_branch: Option<String>,
}

/// 远程分支右键上下文菜单（对齐 IDEA 远程分支菜单）。
pub(crate) fn render_remote_branch_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    target: RemoteBranchMenuTarget,
) -> PopupMenu {
    let RemoteBranchMenuTarget {
        project,
        tab_id,
        remote_branch,
        current_branch,
    } = target;

    let cur = current_branch.unwrap_or_else(|| "main".to_string());
    let app_checkout = app.clone();
    let app_new_branch = app.clone();
    let app_rebase_into = app.clone();
    let app_merge_into = app.clone();
    let app_pull_rebase = app.clone();
    let app_pull_merge = app.clone();
    let app_worktree = app.clone();
    let app_delete = app.clone();
    let app_diff = app.clone();
    let app_compare = app;

    let project_checkout = project.clone();
    let project_new_branch = project.clone();
    let project_rebase = project.clone();
    let project_merge = project.clone();
    let project_pull_rebase = project.clone();
    let project_pull_merge = project.clone();
    let project_worktree = project.clone();
    let project_delete = project;

    let name_checkout = remote_branch.clone();
    let name_new_branch = remote_branch.clone();
    let name_rebase = remote_branch.clone();
    let name_merge = remote_branch.clone();
    let name_pull_rebase = remote_branch.clone();
    let name_pull_merge = remote_branch.clone();
    let name_worktree = remote_branch.clone();
    let name_delete = remote_branch.clone();
    let name_diff = remote_branch.clone();
    let name_compare = remote_branch.clone();

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
                let project = project_new_branch.clone();
                app_new_branch.update(cx, |this, cx| {
                    this.git_create_project_branch_from(&project, &start, window, cx);
                });
            },
        ))
        // 3. 与 '{current}' 比较
        .item(menu_item(
            rust_i18n::t!("git.menu.compare_with_branch", current = cur.as_str()).to_string(),
            IconName::GitCompare,
            move |_, _, cx| {
                let target = name_compare.clone();
                app_compare.update(cx, |this, cx| {
                    this.git_set_log_scope(tab_id, LogScope::Ref(target), cx);
                });
            },
        ))
        // 4. 显示与工作树的差异
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
        .item(PopupMenuItem::separator())
        // 5. 将 '{current}' 变基到 '{remote_branch}'
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
                let project = project_rebase.clone();
                app_rebase_into.update(cx, |this, cx| {
                    this.git_history_on_project(
                        &project,
                        crate::git::actions::GitHistoryAction::Rebase(upstream),
                        cx,
                    );
                });
            },
        ))
        // 6. 将 '{remote_branch}' 合并到 '{current}' 中
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
                let project = project_merge.clone();
                app_merge_into.update(cx, |this, cx| {
                    this.git_history_on_project(
                        &project,
                        crate::git::actions::GitHistoryAction::Merge(reference),
                        cx,
                    );
                });
            },
        ))
        // 7. 从 '{remote_branch}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.new_worktree_from_ref",
                branch = remote_branch.as_str()
            )
            .to_string(),
            IconName::FolderGit,
            move |_, window, cx| {
                let branch = name_worktree.clone();
                let project = project_worktree.clone();
                app_worktree.update(cx, |this, cx| {
                    this.open_git_worktree_modal(project, &branch, window, cx);
                });
            },
        ))
        // 8. 使用“变基”拉入 '{current}'
        .item(menu_item(
            rust_i18n::t!("git.menu.pull_with_rebase", current = cur.as_str()).to_string(),
            IconName::RotateCw,
            move |_, _, cx| {
                let upstream = name_pull_rebase.clone();
                let project = project_pull_rebase.clone();
                app_pull_rebase.update(cx, |this, cx| {
                    this.git_history_on_project(
                        &project,
                        crate::git::actions::GitHistoryAction::Rebase(upstream),
                        cx,
                    );
                });
            },
        ))
        // 9. 使用“合并”拉入 '{current}'
        .item(menu_item(
            rust_i18n::t!("git.menu.pull_with_merge", current = cur.as_str()).to_string(),
            IconName::RotateCw,
            move |_, _, cx| {
                let reference = name_pull_merge.clone();
                let project = project_pull_merge.clone();
                app_pull_merge.update(cx, |this, cx| {
                    this.git_history_on_project(
                        &project,
                        crate::git::actions::GitHistoryAction::Merge(reference),
                        cx,
                    );
                });
            },
        ))
        .item(PopupMenuItem::separator())
        // 10. 删除
        .item(menu_item(
            rust_i18n::t!("git.menu.delete_ref").to_string(),
            IconName::Trash,
            move |_, _, cx| {
                let name = name_delete.clone();
                let project = project_delete.clone();
                app_delete.update(cx, |this, cx| {
                    this.git_delete_project_ref(&project, &name, cx);
                });
            },
        ))
}
